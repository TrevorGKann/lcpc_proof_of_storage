use crate::fields::data_field::DataField;
use crate::fields::field_generator_iter::FieldGeneratorIter;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use crate::lcpc_online::file_handler::write_tree_to_file;
use crate::lcpc_online::merkle_tree::MerkleTree;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;
use std::cmp::{min, Ordering};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::prelude::FileExt;
use std::path::PathBuf;

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
        mut target_file: File,
    ) -> Self {
        let column_digest_accumulator =
            ColumnDigestAccumulator::new(num_encoded_columns, ColumnsToCareAbout::All);
        let incoming_byte_buffer =
            VecDeque::with_capacity(num_encoded_columns * F::DATA_BYTE_CAPACITY as usize * 2);
        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded_columns, num_encoded_columns);
        let num_rows = total_file_size
            .div_ceil(F::DATA_BYTE_CAPACITY as usize)
            .div_ceil(num_pre_encoded_columns);

        // target_file.set_len(total_file_size as u64).unwrap();
        // Self::ensure_file_is_correct_len(&mut target_file, total_file_size as u64);

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

    fn ensure_file_is_correct_len(file: &mut File, desired_len: u64) {
        let current_len = file.metadata().unwrap().len();
        match current_len.cmp(&desired_len) {
            Ordering::Less => {
                // too big of a set_len crashes my computer. I have to write 0 bytes instead
                tracing::debug!("Extending the file to necessary size");
                file.seek(SeekFrom::End(0))
                    .expect("could not seek to end of file");
                let write_buff = [0; 2usize.pow(12)];
                let mut bytes_left = (desired_len - current_len) as usize;
                while bytes_left > 0 {
                    file.write_all(&write_buff[..min(write_buff.len(), bytes_left)])
                        .expect("could not write zero bytes to file");
                    bytes_left -= min(write_buff.len(), bytes_left)
                }
                assert_eq!(file.metadata().unwrap().len(), desired_len);
            }
            Ordering::Equal => {}
            Ordering::Greater => file.set_len(desired_len).unwrap(),
        }
        tracing::debug!("Finished extending the file");
    }

    pub fn convert_unencoded_file(
        unencoded_file: &mut File,
        target_encoded_file: &PathBuf,
        target_digest_file: Option<&mut File>,
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
    ) -> Result<MerkleTree<D>> {
        ensure!(
                num_pre_encoded_columns >= 1,
                "Number of pre-encoded columns must be greater than 0, instead got {num_pre_encoded_columns}"
            );
        ensure!(
            num_encoded_columns >= 2,
            "Number of pencoded columns must be greater than 0, instead got {num_encoded_columns}"
        );
        ensure!(
                num_encoded_columns.is_power_of_two(),
                "Number of encoded columns must be a power of 2, instead got ratio of {num_pre_encoded_columns}/{num_encoded_columns}"
            );
        ensure!(
            num_encoded_columns >= 2 * num_pre_encoded_columns,
            "Number of encoded columns must be greater than 2 * number of columns"
        );

        let total_size = unencoded_file.metadata()?.len() as usize;

        let target_file = File::create(target_encoded_file)?;

        let mut encoded_writer = Self::new(
            num_pre_encoded_columns,
            num_encoded_columns,
            total_size,
            target_file,
        );

        let mut read_buf = [0u8; 2usize.pow(15u32)];
        let mut _total_bytes_read = 0;
        let mut _previous_print_multiple_of_5 = 1.0;
        tracing::trace!(
            "starting encoding of {} bytes of file {}",
            &total_size,
            &target_encoded_file.display()
        );
        loop {
            let bytes_read = unencoded_file.read(&mut read_buf)?;
            if bytes_read == 0 {
                break;
            }

            encoded_writer.push_bytes(&read_buf[..bytes_read])?;

            _total_bytes_read += bytes_read;
            let _percent_done = _total_bytes_read as f64 * 100.0 / total_size as f64;
            if _percent_done / 5.0 > _previous_print_multiple_of_5 {
                tracing::trace!("encoding file: raw file is {}% read", _percent_done);
                _previous_print_multiple_of_5 += 1.0;
            }
        }

        let tree = encoded_writer.finalize_to_merkle_tree()?;

        if let Some(digest_file) = target_digest_file {
            write_tree_to_file::<D>(digest_file, &tree)?;
        }
        Ok(tree)
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes_received += bytes.len();
        ensure!(
            self.bytes_received <= self.total_file_size,
            "too many bytes attempted to be written!"
        );

        self.incoming_byte_buffer.extend(bytes);

        let mut rows_written =
            self.bytes_written / (F::WRITTEN_BYTES_WIDTH as usize * self.encoded_size);
        // process bytes whenever at least a full row has been given
        while self.incoming_byte_buffer.len()
            >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
        {
            self.process_current_row()?;
            rows_written += 1;
            const PERCENT_VALUES: f64 = 25.0;
            if (rows_written as f64 * 100.0 / self.num_rows as f64).floor() % PERCENT_VALUES
                > ((rows_written + 1) as f64 * 100.0 / self.num_rows as f64).floor()
                    % PERCENT_VALUES
            {
                tracing::debug!(
                    "encoding file: file is {}% written",
                    rows_written as f32 * 100.0 / self.num_rows as f32
                );
            }
        }
        Ok(())
    }

    pub fn consume_byte_iterator(
        &mut self,
        byte_iterator: &mut impl Iterator<Item = u8>,
    ) -> Result<()> {
        while let Some(byte) = byte_iterator.next() {
            self.incoming_byte_buffer.push_back(byte);
            self.bytes_received += 1;

            while self.incoming_byte_buffer.len()
                >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
            {
                self.process_current_row()?;
            }
        }
        Ok(())
    }

    /// Don't call on an unfinished row unless it is the last row that will never be fully filled.
    ///  Otherwise, the file can be corrupted
    fn process_current_row(&mut self) -> Result<()> {
        let encoded_row = self.encode_current_row()?;

        // update digests
        self.column_digest_accumulator.update(&encoded_row)?;

        // write sparse file
        self.write_row(&encoded_row)?;

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
        assert!(!row_to_encode.is_empty(), "should not encode empty row");
        assert!(
            row_to_encode.len() <= self.pre_encoded_size,
            "too many elements were taken to encode"
        );

        row_to_encode
            .extend(std::iter::repeat(F::ZERO).take(self.encoded_size - row_to_encode.len()));

        assert_eq!(
            row_to_encode.len(),
            self.encoded_size,
            "encoded row setup is not the correct size to encode"
        );
        self.encoding.encode(&mut row_to_encode).unwrap();
        assert_eq!(
            row_to_encode.len(),
            self.encoded_size,
            "encoded row is not the correct size"
        );
        Ok(row_to_encode)
    }

    fn write_row(&mut self, encoded_row: &[F]) -> Result<()> {
        // let row_bytes = self
        //     .incoming_byte_buffer
        //     .range(0..min(self.pre_encoded_size, self.incoming_byte_buffer.len()));
        let row_bytes: Vec<u8> = F::field_vec_to_raw_bytes(encoded_row);
        assert_eq!(
            row_bytes.len(),
            self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize,
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

        let mut write_location = (rows_written * F::WRITTEN_BYTES_WIDTH as usize) as u64;
        if !cfg!(unix) {
            // windows doesn't have `write_at` atomics, so we need to seek around separately
            self.file_to_write_to
                .seek(SeekFrom::Start(write_location))?;
        }

        let column_length_in_bytes = self.num_rows as i64 * F::WRITTEN_BYTES_WIDTH as i64;
        for bytes_of_field_element in bytes_to_write_iterator.into_iter() {
            if cfg!(unix) {
                self.file_to_write_to
                    .write_at(bytes_of_field_element, write_location)?;
                write_location += column_length_in_bytes as u64;
            } else {
                self.file_to_write_to.write_all(&bytes_of_field_element)?;
                // self.file_to_write_to.flush()?;
                self.file_to_write_to.seek(SeekFrom::Current(
                    column_length_in_bytes - F::WRITTEN_BYTES_WIDTH as i64,
                ))?;
            }
            // self.bytes_written += F::WRITTEN_BYTES_WIDTH as usize;
            self.bytes_written += bytes_of_field_element.len();
        }
        Ok(())
    }

    pub fn finalize_to_column_digest(mut self) -> Result<Vec<Output<D>>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row()?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        Ok(self.column_digest_accumulator.get_column_digests())
    }

    pub fn finalize_to_commit(mut self) -> Result<Output<D>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row()?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        self.column_digest_accumulator.finalize_to_commit() //unwrap won't panic because we are using Columns::All
    }

    pub fn finalize_to_merkle_tree(mut self) -> Result<MerkleTree<D>> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row()?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        self.column_digest_accumulator.finalize_to_merkle_tree()
    }
}
