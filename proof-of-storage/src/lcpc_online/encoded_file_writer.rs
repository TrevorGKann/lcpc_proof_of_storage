use crate::fields::data_field::DataField;
use crate::fields::field_generator_iter::FieldGeneratorIter;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use anyhow::Result;
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use itertools::Itertools;
use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;
use std::cmp::min;
use std::collections::VecDeque;
use std::io::SeekFrom;
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

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
        let incoming_byte_buffer = VecDeque::with_capacity(num_encoded_columns * 2);
        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded_columns, num_encoded_columns);

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
            num_rows: total_file_size / num_encoded_columns,
        }
    }

    pub async fn push_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.incoming_byte_buffer.extend(bytes);

        while self.incoming_byte_buffer.len() >= self.pre_encoded_size {
            self.process_current_row().await?;
            todo!()
        }
        Ok(())
    }

    /// Don't call on an unfinished row unless it is the last row that will never be fully filled.
    ///  Otherwise, the file can be corrupted
    async fn process_current_row(&mut self) -> Result<()> {
        let encoded_row = self.encode_current_row();

        // update digests
        self.column_digest_accumulator.update(&encoded_row).unwrap();

        // write sparse file
        self.write_row().await?;

        self.drain_current_row();

        Ok(())
    }

    fn drain_current_row(&mut self) {
        self.incoming_byte_buffer.drain(0..self.pre_encoded_size);
    }

    fn encode_current_row(&mut self) -> Vec<F> {
        let bytes_to_encode_iterator = &mut self
            .incoming_byte_buffer
            .range(0..min(self.pre_encoded_size, self.incoming_byte_buffer.len()));
        let mut row_to_encode: Vec<F> = Vec::with_capacity(self.encoded_size);
        row_to_encode.extend(FieldGeneratorIter::<_, F>::new(
            bytes_to_encode_iterator.cloned(),
        ));

        row_to_encode
            .extend(std::iter::repeat(F::ZERO).take(self.encoded_size - row_to_encode.len()));

        self.encoding.encode(&mut row_to_encode);

        row_to_encode
    }

    async fn write_row(&mut self) -> Result<()> {
        let row_bytes = self
            .incoming_byte_buffer
            .range(0..min(self.pre_encoded_size, self.incoming_byte_buffer.len()));

        let bytes_to_write_iterator = row_bytes.chunks(F::DATA_BYTE_CAPACITY as usize);
        let field_elements_written = self.bytes_written / F::DATA_BYTE_CAPACITY as usize;
        self.file_to_write_to
            .seek(SeekFrom::Start(
                (field_elements_written / self.encoded_size) as u64,
            ))
            .await?;
        for byte_of_field_elements in bytes_to_write_iterator.into_iter() {
            self.file_to_write_to
                .write_all(&byte_of_field_elements.cloned().collect::<Vec<u8>>())
                .await?;
            self.file_to_write_to
                .seek(SeekFrom::Current(
                    F::DATA_BYTE_CAPACITY as i64 * self.num_rows as i64,
                ))
                .await?;
        }
        Ok(())
    }

    pub fn finalize_to_column_digest(mut self) -> Vec<Output<D>> {
        if self.incoming_byte_buffer.len() > 0 {}
        self.column_digest_accumulator.get_column_digests()
    }

    pub async fn finalize_to_commit(mut self) -> Result<Output<D>> {
        if self.incoming_byte_buffer.len() > 0 {
            self.process_current_row().await?
        }
        Ok(self.column_digest_accumulator.finalize_to_commit().unwrap()) //unwrap won't panic because we are using Columns::All
    }
}
