use std::cmp::{min, Ordering};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::prelude::FileExt;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use rayon::iter::IndexedParallelIterator;
use rayon::iter::ParallelIterator;
use rayon::prelude::{IntoParallelIterator, IntoParallelRefMutIterator};

use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;

use crate::fields::data_field::DataField;
use crate::fields::field_generator_iter::FieldGeneratorIter;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use crate::lcpc_online::file_handler::write_tree_to_file;
use crate::lcpc_online::merkle_tree::MerkleTree;
use crate::lcpc_online::EncodedFileMetadata;

pub struct EncodedFileWriter<F: DataField, D: Digest + FixedOutputReset + Send, E: LcEncoding> {
    encoding: E,
    column_digest_accumulator: ColumnDigestAccumulator<D, F>,
    incoming_byte_buffer: VecDeque<u8>,
    outgoing_field_buffer_size: usize,
    outgoing_field_buffer: Vec<Vec<F>>,
    bytes_received: usize,
    bytes_written: usize,
    rows_processed: usize,
    rows_written: usize,
    file_to_write_to: File,
    pre_encoded_size: usize,
    encoded_size: usize,
    row_capacity: usize,
}

impl<F: DataField, D: Digest + FixedOutputReset + Send, E: LcEncoding> EncodedFileWriter<F, D, E> {
    pub fn get_encoded_file_metadata(&self) -> EncodedFileMetadata {
        EncodedFileMetadata {
            ulid: Default::default(),
            pre_encoded_size: self.pre_encoded_size,
            encoded_size: self.encoded_size,
            rows_written: self.rows_written,
            row_capacity: self.row_capacity,
            bytes_of_data: self.bytes_received, // different because data_bytes != bytes_written
        }
    }
}

impl<F: DataField, D: Digest + FixedOutputReset + Send> EncodedFileWriter<F, D, LigeroEncoding<F>> {
    pub fn new(
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
        original_file_size: usize,
        target_file: File,
    ) -> Self {
        assert!(
            num_encoded_columns.is_power_of_two(),
            "num_encoded_columns must be a power of two"
        );
        assert!(
            num_pre_encoded_columns < num_encoded_columns,
            "num_pre_encoded_columns must be less than num_encoded_columns"
        );
        assert!(
            num_pre_encoded_columns > 0,
            "num_pre_encoded_columns must be > 0"
        );

        let column_digest_accumulator =
            ColumnDigestAccumulator::new(num_encoded_columns, ColumnsToCareAbout::All);
        let incoming_byte_buffer =
            VecDeque::with_capacity(num_encoded_columns * F::DATA_BYTE_CAPACITY as usize * 2);
        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded_columns, num_encoded_columns);
        let num_rows = original_file_size
            .div_ceil(F::DATA_BYTE_CAPACITY as usize)
            .div_ceil(num_pre_encoded_columns);
        let row_capacity = num_rows * 2;

        // todo: probably parameterize this at some point in some way
        let per_column_buffer_size = min(2usize.pow(10), num_rows);
        let outgoing_field_buffer =
            vec![Vec::with_capacity(per_column_buffer_size); num_encoded_columns];

        // preallocate file as zeros so we don't need to live request more blocks on the fly
        // Self::ensure_file_is_correct_len(&mut target_file, total_file_size as u64);
        let desired_encoded_len =
            row_capacity * num_encoded_columns * F::WRITTEN_BYTES_WIDTH as usize;
        target_file.set_len(desired_encoded_len as _).unwrap();

        EncodedFileWriter {
            encoding,
            column_digest_accumulator,
            incoming_byte_buffer,
            outgoing_field_buffer,
            outgoing_field_buffer_size: per_column_buffer_size,
            bytes_received: 0,
            rows_processed: 0,
            bytes_written: 0,
            file_to_write_to: target_file,
            pre_encoded_size: num_pre_encoded_columns,
            encoded_size: num_encoded_columns,
            row_capacity,
            rows_written: 0,
        }
    }

    fn _ensure_file_is_correct_len(file: &mut File, desired_len: u64) {
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
        target_digest_file: Option<&Path>,
        target_metadata_file: Option<&Path>,
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
    ) -> Result<(EncodedFileMetadata, MerkleTree<D>)> {
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
            num_encoded_columns > num_pre_encoded_columns,
            "Number of encoded columns must be greater than the number of columns"
        );

        let total_size = unencoded_file.metadata()?.len() as usize;

        let target_file = File::create(target_encoded_file)?;

        let mut encoded_writer = Self::new(
            num_pre_encoded_columns,
            num_encoded_columns,
            total_size,
            target_file,
        );

        let mut read_buf = [0u8; 2usize.pow(15)];
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

        let (metadata, tree) = encoded_writer.finalize_to_merkle_tree()?;
        assert_eq!(_total_bytes_read, metadata.bytes_of_data);
        assert_eq!(
            _total_bytes_read.div_ceil(F::DATA_BYTE_CAPACITY as usize * num_pre_encoded_columns),
            metadata.rows_written
        );
        if let Some(metadata_file_path) = target_metadata_file {
            let mut metadata_file = File::create(metadata_file_path)?;
            metadata.write_to_file(&mut metadata_file)?;
        }

        if let Some(digest_file_path) = target_digest_file {
            let mut digest_file = File::create(digest_file_path)?;
            write_tree_to_file::<D>(&mut digest_file, &tree)?;
        }
        Ok((metadata, tree))
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes_received += bytes.len();
        // ensure!(
        //     self.bytes_received <= self.total_file_size,
        //     "too many bytes attempted to be written!"
        // );

        self.incoming_byte_buffer.extend(bytes);

        // let mut rows_processed =
        //     self.bytes_written / (F::WRITTEN_BYTES_WIDTH as usize * self.encoded_size);
        // process bytes whenever at least a full row has been given
        while self.incoming_byte_buffer.len()
            >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
        {
            self.process_current_row(false)?;
            const PERCENT_VALUES: f64 = 25.0;
            if (self.rows_processed as f64 * 100.0 / self.rows_written as f64).floor()
                % PERCENT_VALUES
                > ((self.rows_processed + 1) as f64 * 100.0 / self.rows_written as f64).floor()
                    % PERCENT_VALUES
            {
                tracing::debug!(
                    "encoding file: file is {}% processed",
                    self.rows_processed as f32 * 100.0 / self.rows_written as f32
                );
            }
        }
        Ok(())
    }

    pub fn consume_byte_iterator(
        &mut self,
        byte_iterator: &mut impl Iterator<Item = u8>,
    ) -> Result<()> {
        for byte in byte_iterator.by_ref() {
            self.incoming_byte_buffer.push_back(byte);
            self.bytes_received += 1;

            while self.incoming_byte_buffer.len()
                >= (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize)
            {
                self.process_current_row(false)?;
            }
        }
        Ok(())
    }

    /// Don't call on an unfinished row unless it is the last row that will never be fully filled.
    ///  Otherwise, the file can be corrupted
    fn process_current_row(&mut self, finalize: bool) -> Result<()> {
        let encoded_row = self.encode_current_row()?;

        // update digests
        self.column_digest_accumulator.update(&encoded_row)?;

        // write sparse file
        // self.write_row(encoded_row)?;
        self.buffered_write_row(encoded_row, finalize)?;

        self.rows_processed += 1;

        Ok(())
    }

    fn encode_current_row(&mut self) -> Result<Vec<F>> {
        let drain_target = min(
            self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize,
            self.incoming_byte_buffer.len(),
        );
        self.bytes_written += drain_target;
        let bytes_to_encode_iterator = self.incoming_byte_buffer.drain(0..drain_target);
        let mut row_to_encode: Vec<F> = Vec::with_capacity(self.encoded_size);
        row_to_encode.extend(FieldGeneratorIter::<_, F>::new(bytes_to_encode_iterator));
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

    fn buffered_write_row(&mut self, encoded_row: Vec<F>, finalize: bool) -> Result<()> {
        (encoded_row, &mut self.outgoing_field_buffer)
            .into_par_iter()
            .for_each(|(incoming_field_element, column_buffer)| {
                column_buffer.push(incoming_field_element)
            });
        let current_buf_size = self.outgoing_field_buffer.first().unwrap().len();

        // if there's no reason to drain the buffer to the file then return
        if current_buf_size < self.outgoing_field_buffer_size // buff isn't filled
            && current_buf_size + self.rows_written < self.row_capacity // we aren't going to overflow
            && !finalize
        // optimization: this might be called up to rowbuf times
        {
            return Ok(());
        }

        // otherwise the buffer is full or would complete the file
        let write_location = (self.rows_written * F::WRITTEN_BYTES_WIDTH as usize) as u64;
        #[cfg(not(unix))]
        self.file_to_write_to
            .seek(SeekFrom::Start(write_location))?;

        let column_length_in_bytes = self.row_capacity as u64 * F::WRITTEN_BYTES_WIDTH as u64;
        // let pool = rayon::ThreadPoolBuilder::new()
        //     .num_threads(64)
        //     .build()
        //     .expect("couldn't build thread pool");
        // pool.install(|| {
        self.outgoing_field_buffer
            .par_iter_mut()
            .enumerate()
            .try_for_each(|(row_index, f): (usize, &mut Vec<F>)| {
                let bytes = F::field_vec_to_raw_bytes(f);
                f.clear();
                #[cfg(unix)]
                {
                    self.file_to_write_to.write_at(
                        &bytes,
                        write_location + column_length_in_bytes * row_index as u64,
                    )?;
                    // no matter how many bytes we write, the next location to write
                    // to will always be a column_length away since `write_at` doesn't
                    // affect the file pointer. This is not the case for write
                    // operations other than `write_at`
                }
                #[cfg(not(unix))]
                {
                    todo!("Windows write at implementation not yet complete")
                }
                anyhow::Ok(())
            })
            // })
            ?;
        // for bytes_from_column_buff_to_write in bytes_of_columns {
        //     #[cfg(unix)]
        //     {
        //         self.file_to_write_to
        //             .write_at(&bytes_from_column_buff_to_write, write_location)?;
        //         write_location += column_length_in_bytes as u64;
        //         // no matter how many bytes we write, the next location to write
        //         // to will always be a column_length away since `write_at` doesn't
        //         // affect the file pointer. This is not the case for write
        //         // operations other than `write_at`
        //     }
        //     #[cfg(not(unix))]
        //     {
        //         todo!() // todo: I can figure this out later
        //     }
        //     self.bytes_written += bytes_from_column_buff_to_write.len();
        // }
        self.rows_written += current_buf_size;
        // self.bytes_written += current_buf_size * self.outgoing_field_buffer.len();

        assert!(self.rows_written <= self.row_capacity);
        if self.rows_written == self.row_capacity {
            self.set_new_capacity(self.row_capacity * 2)?;
        }

        Ok(())
    }

    fn _write_row(&mut self, encoded_row: Vec<F>) -> Result<()> {
        let row_bytes: Vec<u8> = F::field_vec_to_raw_bytes(&encoded_row);
        assert_eq!(
            row_bytes.len(),
            self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize,
            "wrong number of bytes to write to file"
        );

        let bytes_to_write_iterator = row_bytes.chunks(F::WRITTEN_BYTES_WIDTH as usize);
        let field_elements_written = self.bytes_written / F::WRITTEN_BYTES_WIDTH as usize;
        let rows_written = field_elements_written / self.encoded_size;

        let mut write_location = (rows_written * F::WRITTEN_BYTES_WIDTH as usize) as u64;
        if !cfg!(unix) {
            // windows doesn't have `write_at` atomics, so we need to seek around separately
            self.file_to_write_to
                .seek(SeekFrom::Start(write_location))?;
        }

        let column_length_in_bytes = self.row_capacity as i64 * F::WRITTEN_BYTES_WIDTH as i64;
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

    pub fn set_new_capacity(&mut self, new_capacity: usize) -> Result<()> {
        ensure!(
            new_capacity >= self.rows_written,
            "Cannot set capacity to fewer than rows are written. That would be destructive!"
        );

        self.file_to_write_to.set_len(
            new_capacity as u64 * self.encoded_size as u64 * F::WRITTEN_BYTES_WIDTH as u64,
        )?;

        // optimization: right now I'm gonna do it single threaded and in place but in the future it could be not
        //  in-place and multi-threaded. However, we don't need to optimize that for the paper :D

        let old_column_length = self.row_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let new_column_length = new_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let mut read_in_buffer = vec![0u8; old_column_length];
        let write_out_pad = vec![0u8; new_column_length - old_column_length];

        for row in 0..self.rows_written {
            // pick up old rows starting from the last one
            let old_row_start = row * old_column_length;
            self.file_to_write_to
                .read_at(&mut read_in_buffer, old_row_start as _)?;

            // place them in their final destination and flush out an entire row
            let new_row_start = row * new_column_length;
            self.file_to_write_to
                .write_at(&read_in_buffer, new_row_start as _)?;
            self.file_to_write_to
                .write_at(&write_out_pad, (new_row_start + old_column_length) as _)?;
        }

        Ok(())
    }

    pub fn finalize_to_column_digest(mut self) -> Result<(EncodedFileMetadata, Vec<Output<D>>)> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row(true)?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        let metadata = self.get_encoded_file_metadata();
        let digests = self.column_digest_accumulator.get_column_digests();

        Ok((metadata, digests))
    }

    pub fn finalize_to_commit(mut self) -> Result<(EncodedFileMetadata, Output<D>)> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row(true)?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        let metadata = self.get_encoded_file_metadata();
        let commit = self.column_digest_accumulator.finalize_to_commit()?;
        Ok((metadata, commit))
    }

    pub fn finalize_to_merkle_tree(mut self) -> Result<(EncodedFileMetadata, MerkleTree<D>)> {
        while !self.incoming_byte_buffer.is_empty() {
            self.process_current_row(true)?
        }
        assert!(
            self.incoming_byte_buffer.is_empty(),
            "incoming byte buffer is not yet empty, shouldn't be finalizing"
        );

        let metadata = self.get_encoded_file_metadata();

        let tree = self.column_digest_accumulator.finalize_to_merkle_tree()?;

        Ok((metadata, tree))
    }
}
