use crate::fields::data_field::DataField;
use crate::fields::field_generator_iter::FieldGeneratorIter;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use crate::lcpc_online::file_handler::write_tree_to_file;
use crate::lcpc_online::merkle_tree::MerkleTree;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;
use std::cmp::min;
use std::collections::VecDeque;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

pub struct EncodedFileWriter<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding> {
    encoding: E,
    column_digest_accumulator: ColumnDigestAccumulator<D, F>,
    incoming_byte_buffer: VecDeque<u8>,
    total_file_size: usize,
    bytes_received: usize,
    bytes_written: usize,
    file_to_write_to: File,
    pre_encoded_size: usize,
    encoded_size: usize,
    num_rows: usize,
}

impl<F: DataField, D: Digest + FixedOutputReset> EncodedFileWriter<F, D, LigeroEncoding<F>> {
    pub fn new(
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
        total_file_size: usize,
        target_file: File,
    ) -> Self {
        let column_digest_accumulator =
            ColumnDigestAccumulator::new(num_encoded_columns, ColumnsToCareAbout::All);
        let incoming_byte_buffer =
            VecDeque::with_capacity(num_encoded_columns * F::DATA_BYTE_CAPACITY as usize * 2);
        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded_columns, num_encoded_columns);
        let num_rows = total_file_size
            .div_ceil(F::DATA_BYTE_CAPACITY as usize)
            .div_ceil(num_pre_encoded_columns);

        EncodedFileWriter {
            encoding,
            column_digest_accumulator,
            incoming_byte_buffer,
            total_file_size,
            bytes_received: 0,
            bytes_written: 0,
            file_to_write_to: target_file,
            pre_encoded_size: num_pre_encoded_columns,
            encoded_size: num_encoded_columns,
            num_rows,
        }
    }

    pub async fn convert_unencoded_file(
        unencoded_file: &mut File,
        target_encoded_file: &PathBuf,
        target_digest_file: Option<&mut File>,
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
    ) -> Result<MerkleTree<D>> {
        let total_size = unencoded_file.metadata().await?.len() as usize;

        unencoded_file.seek(SeekFrom::Start(0)).await?;
        let target_file = OpenOptions::default()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&target_encoded_file)
            .await?;

        let mut encoded_writer = Self::new(
            num_pre_encoded_columns,
            num_encoded_columns,
            total_size,
            target_file,
        );

        let mut read_buf = [0u8; 4098];
        loop {
            let bytes_read = unencoded_file.read(&mut read_buf).await?;
            if bytes_read == 0 {
                break;
            }
            encoded_writer.push_bytes(&read_buf[..bytes_read]).await?;
        }

        let tree = encoded_writer.finalize_to_merkle_tree().await?;

        if let Some(digest_file) = target_digest_file {
            write_tree_to_file::<D>(digest_file, &tree).await?;
        }
        Ok(tree)
    }

    pub async fn push_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes_received += bytes.len();
        ensure!(
            self.bytes_received <= self.total_file_size,
            "too many bytes attempted to be written!"
        );

        self.incoming_byte_buffer.extend(bytes);

        // process bytes whenever at least a full row has been given
        while self.incoming_byte_buffer.len()
            >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
        {
            self.process_current_row().await?;
        }
        Ok(())
    }

    pub async fn consume_byte_iterator(
        &mut self,
        byte_iterator: &mut impl Iterator<Item = u8>,
    ) -> Result<()> {
        while let Some(byte) = byte_iterator.next() {
            self.incoming_byte_buffer.push_back(byte);
            self.bytes_received += 1;

            while self.incoming_byte_buffer.len()
                >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
            {
                self.process_current_row().await?;
            }
        }
        Ok(())
    }

    /// Don't call on an unfinished row unless it is the last row that will never be fully filled.
    ///  Otherwise, the file can be corrupted
    async fn process_current_row(&mut self) -> Result<()> {
        let encoded_row = self.encode_current_row()?;

        // update digests
        self.column_digest_accumulator.update(&encoded_row)?;

        // write sparse file
        self.write_row(&encoded_row).await?;

        Ok(())
    }

    fn encode_current_row(&mut self) -> Result<Vec<F>> {
        let drain_target = min(
            self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize,
            self.incoming_byte_buffer.len(),
        );
        let bytes_to_encode_iterator = self.incoming_byte_buffer.drain(0..drain_target);
        let mut row_to_encode: Vec<F> = Vec::with_capacity(self.encoded_size);
        row_to_encode.extend(FieldGeneratorIter::<_, F>::new(
            bytes_to_encode_iterator, // optimization: shouldn't have to be cloned but it is atm
        ));
        ensure!(!row_to_encode.is_empty(), "should not encode empty row");
        ensure!(
            row_to_encode.len() <= self.pre_encoded_size,
            "too many elements were taken to encode"
        );

        row_to_encode
            .extend(std::iter::repeat(F::ZERO).take(self.encoded_size - row_to_encode.len()));

        ensure!(
            row_to_encode.len() == self.encoded_size,
            "encoded row setup is not the correct size to encode"
        );
        self.encoding.encode(&mut row_to_encode).unwrap();
        ensure!(
            row_to_encode.len() == self.encoded_size,
            "encoded row is not the correct size"
        );
        Ok(row_to_encode)
    }

    async fn write_row(&mut self, encoded_row: &[F]) -> Result<()> {
        // let row_bytes = self
        //     .incoming_byte_buffer
        //     .range(0..min(self.pre_encoded_size, self.incoming_byte_buffer.len()));
        let row_bytes: Vec<u8> = F::field_vec_to_raw_bytes(encoded_row);
        ensure!(
            row_bytes.len() == self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize,
            "wrong number of bytes to write to file"
        );

        let bytes_to_write_iterator = row_bytes.chunks(F::WRITTEN_BYTES_WIDTH as usize);
        let field_elements_written = self.bytes_written / F::WRITTEN_BYTES_WIDTH as usize;
        let rows_written = field_elements_written / self.encoded_size;
        ensure!(
            rows_written < self.num_rows,
            "attempting to write more rows than expected"
        );
        // todo: probably will remove this upon optimization

        self.file_to_write_to
            .seek(SeekFrom::Start(
                (rows_written * F::WRITTEN_BYTES_WIDTH as usize) as u64,
            ))
            .await?;

        let column_length_in_bytes = self.num_rows as i64 * F::WRITTEN_BYTES_WIDTH as i64;
        for bytes_of_field_element in bytes_to_write_iterator.into_iter() {
            self.file_to_write_to
                .write_all(&bytes_of_field_element)
                .await?;
            self.file_to_write_to.flush().await?;
            self.file_to_write_to
                .seek(SeekFrom::Current(
                    column_length_in_bytes - F::WRITTEN_BYTES_WIDTH as i64,
                ))
                .await?;

            // self.bytes_written += F::WRITTEN_BYTES_WIDTH as usize;
            self.bytes_written += bytes_of_field_element.len();
        }
        Ok(())
    }

    pub async fn finalize_to_column_digest(mut self) -> Result<Vec<Output<D>>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row().await?
        }
        ensure!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        self.file_to_write_to.flush().await?;
        self.file_to_write_to.sync_all().await?;

        Ok(self.column_digest_accumulator.get_column_digests())
    }

    pub async fn finalize_to_commit(mut self) -> Result<Output<D>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row().await?
        }
        ensure!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        self.file_to_write_to.flush().await?;
        self.file_to_write_to.sync_all().await?;

        self.column_digest_accumulator.finalize_to_commit() //unwrap won't panic because we are using Columns::All
    }

    pub async fn finalize_to_merkle_tree(mut self) -> Result<MerkleTree<D>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row().await?
        }
        ensure!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        self.file_to_write_to.flush().await?;
        self.file_to_write_to.sync_all().await?;

        self.column_digest_accumulator.finalize_to_merkle_tree()
    }
}
