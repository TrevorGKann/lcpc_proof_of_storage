use std::cmp::{max, min};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::os::unix::prelude::FileExt;

use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use rayon::iter::repeatn;
use rayon::prelude::*;

use lcpc_2d::{FieldHash, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;

use crate::fields::data_field::DataField;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use crate::lcpc_online::merkle_tree::MerkleTree;
use crate::lcpc_online::{decode_row, EncodedFileMetadata};

pub struct EncodedFileReader<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>> {
    encoding: E,
    // total_file_size: usize,
    file_to_read: File,
    pre_encoded_size: usize,
    encoded_size: usize,
    rows_written: usize,
    row_capacity: usize,
    _field_type: PhantomData<F>,
    _digest_type: PhantomData<D>,
}

impl<F: DataField, D: Digest + FixedOutputReset + Send + Sync>
    EncodedFileReader<F, D, LigeroEncoding<F>>
{
    pub fn new_ligero(
        file_to_read: File,
        pre_encoded_size: usize,
        encoded_size: usize,
        rows_written: usize,
        row_capacity: usize,
    ) -> Self {
        let encoding = LigeroEncoding::<F>::new_from_dims(pre_encoded_size, encoded_size);
        // let total_file_size = file_to_read.metadata().unwrap().len() as usize;
        // let row_capacity = total_file_size.div_ceil(encoded_size * F::WRITTEN_BYTES_WIDTH as usize);

        Self {
            encoding,
            // total_file_size,
            file_to_read,
            pre_encoded_size,
            encoded_size,
            rows_written,
            row_capacity,
            _field_type: PhantomData,
            _digest_type: PhantomData,
        }
    }

    pub fn get_unencoded_row(&mut self, target_row: usize) -> Result<Vec<F>> {
        ensure!(
            target_row < self.rows_written,
            "target row index is out of bounds"
        );

        let encoded_row = self.get_encoded_row(target_row)?;
        let mut decoded_row = decode_row(encoded_row)?;
        decoded_row.drain(self.pre_encoded_size..);

        ensure!(decoded_row.len() == self.pre_encoded_size);
        Ok(decoded_row)
    }

    pub fn get_unencoded_row_bytes(&mut self, target_row: usize) -> Result<Vec<u8>> {
        let row_bytes = F::field_vec_to_byte_vec(&self.get_unencoded_row(target_row)?);
        ensure!(row_bytes.len() == self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        Ok(row_bytes)
    }

    pub fn decode_to_target_file(&mut self, target_file: &mut File) -> Result<()> {
        for row_i in 0..self.rows_written {
            // let decoded_row = self.get_unencoded_row_bytes(row_i)?
            let encoded_row = self.get_encoded_row(row_i)?;
            let mut decoded_row = decode_row(encoded_row)?;
            decoded_row.drain(self.pre_encoded_size..);
            assert_eq!(decoded_row.len(), self.pre_encoded_size);
            let decoded_bytes = F::field_vec_to_byte_vec(&decoded_row);
            target_file.write_all(&decoded_bytes)?;
        }
        target_file.flush()?;
        Ok(())
    }

    pub fn get_unencoded_file_len(&self) -> Result<usize> {
        Ok(self.file_to_read.metadata()?.len() as usize
            / (self.encoded_size / self.pre_encoded_size))
    }

    pub fn resize_to_target_file(
        &mut self,
        target_file: File,
        new_pre_encoded_size: usize,
        new_encoded_size: usize,
    ) -> Result<(EncodedFileMetadata, MerkleTree<D>)> {
        let mut new_encoded_file_writer = EncodedFileWriter::<F, D, LigeroEncoding<F>>::new(
            new_pre_encoded_size,
            new_encoded_size,
            self.get_unencoded_file_len()?,
            target_file,
        );

        for row_i in 0..self.rows_written {
            new_encoded_file_writer.push_bytes(self.get_unencoded_row_bytes(row_i)?.as_slice())?;
        }

        new_encoded_file_writer.finalize_to_merkle_tree()
    }

    /// returns the original data that was replaced in the unencoded file
    pub fn edit_decoded_bytes(
        &mut self,
        unencoded_start_byte: usize,
        new_unencoded_data: &[u8],
    ) -> Result<Vec<u8>> {
        // ensure!(
        //     unencoded_start_byte + new_unencoded_data.len() <= self.total_file_size,
        //     "can't edit past the end of the file"
        // );

        let unencoded_start_field_element = unencoded_start_byte / F::DATA_BYTE_CAPACITY as usize;
        let unencoded_end_field_element = (unencoded_start_byte + new_unencoded_data.len())
            .div_ceil(F::DATA_BYTE_CAPACITY as usize);

        let start_row = unencoded_start_field_element / self.pre_encoded_size;
        let end_row = unencoded_end_field_element.div_ceil(self.pre_encoded_size);

        // optimization: this is way overcomplicated, I already updated the bytes in the original file,
        //  I can just iteratively read rows and then buffer write them back.
        let mut original_unencoded_bytes_result: Vec<u8> =
            Vec::with_capacity(new_unencoded_data.len());
        let mut current_byte = unencoded_start_byte;
        let end_byte = current_byte + new_unencoded_data.len();
        let mut bytes_written = 0;
        for row_index in start_row..end_row {
            assert!(
                current_byte < end_byte,
                "Math error: current byte shouldn't be larger than last within loop"
            );
            let mut original_bytes = self.get_unencoded_row_bytes(row_index)?;

            let start_of_row_byte_index =
                row_index * self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize;
            let end_of_row_byte_index =
                (row_index + 1) * self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize;

            let start_of_bytes_to_care_for = max(current_byte, start_of_row_byte_index)
                % (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
            // we want to map end_of_bytes to 1-14 instead of 0-13, like a usual mod does, so we have this weirdism
            let end_of_bytes_to_care_for = min(end_byte, end_of_row_byte_index)
                % (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
            let end_of_bytes_to_care_for = if end_of_bytes_to_care_for == 0 {
                self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize
            } else {
                end_of_bytes_to_care_for
            };

            // keep track of the original bytes that were replaced to return at the end
            original_unencoded_bytes_result.extend_from_slice(
                &original_bytes[start_of_bytes_to_care_for..end_of_bytes_to_care_for],
            );

            // replace the bytes in the original bytes with the new bytes to later encode into the row
            original_bytes[start_of_bytes_to_care_for..end_of_bytes_to_care_for].copy_from_slice(
                &new_unencoded_data[bytes_written
                    ..bytes_written + end_of_bytes_to_care_for - start_of_bytes_to_care_for],
            );
            bytes_written += end_of_bytes_to_care_for - start_of_bytes_to_care_for;
            // for byte_index in start_of_bytes_to_care_for..end_of_bytes_to_care_for {
            //     original_bytes[byte_index] = new_unencoded_data[bytes_written];
            //     bytes_written += 1;
            // }

            let mut new_row = F::from_byte_vec(&original_bytes);
            assert_eq!(new_row.len(), self.pre_encoded_size);
            // new_row.par_extend(repeat_with(|| F::ZERO).take(self.encoded_size - new_row.len()));
            new_row.par_extend(repeatn(F::ZERO, self.encoded_size - new_row.len()));
            assert_eq!(new_row.len(), self.encoded_size);
            self.encoding.encode(&mut new_row)?;
            self.replace_encoded_row(row_index, &new_row)?;

            current_byte += end_of_bytes_to_care_for - start_of_bytes_to_care_for;
        }

        Ok(original_unencoded_bytes_result)
    }

    pub fn replace_row_with_decoded_bytes(
        &mut self,
        row_index: usize,
        new_unencoded_row: &[u8],
    ) -> Result<()> {
        let mut row_to_encode: Vec<F> = Vec::with_capacity(self.encoded_size);
        row_to_encode.extend_from_slice(&F::from_byte_vec(new_unencoded_row));
        row_to_encode.par_extend(repeatn(F::ZERO, self.encoded_size - row_to_encode.len()));
        self.encoding.encode(&mut row_to_encode)?;
        self.replace_encoded_row(row_index, &row_to_encode)?;
        Ok(())
    }
}

// generic over all encodings
impl<F: DataField, D: Digest + FixedOutputReset + Send + Sync, E: LcEncoding<F = F>>
    EncodedFileReader<F, D, E>
{
    pub fn get_encoded_row(&mut self, target_row: usize) -> Result<Vec<F>> {
        // let mut row_start_byte = target_row * self.num_rows * F::WRITTEN_BYTES_WIDTH as usize;
        let row_start_byte = target_row * F::WRITTEN_BYTES_WIDTH as usize;
        let bytes_per_row = self.row_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let bytes_per_column = self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize;
        let mut encoded_row_bytes: Vec<u8> = Vec::with_capacity(bytes_per_column);
        let mut file_pointer = row_start_byte as u64;
        if !cfg!(unix) {
            self.file_to_read.seek(SeekFrom::Start(file_pointer))?;
        }

        // self.file_to_read.read_exact(&mut encoded_row_bytes)?;

        for i in 0..self.encoded_size {
            let mut bytes_to_read = vec![0u8; F::WRITTEN_BYTES_WIDTH as usize];
            if cfg!(unix) {
                self.file_to_read
                    .read_exact_at(&mut bytes_to_read, file_pointer)?;
            } else {
                self.file_to_read.read_exact(&mut bytes_to_read)?;
            }
            encoded_row_bytes.extend_from_slice(&bytes_to_read);

            // on all but the last-read column as not to seek past end of the file
            if i < self.encoded_size - 1 {
                if cfg!(unix) {
                    file_pointer += bytes_per_row as u64
                } else {
                    self.file_to_read.seek(SeekFrom::Current(
                        bytes_per_row as i64 - F::WRITTEN_BYTES_WIDTH as i64,
                        // since reading advances the file pointer by one F::WRITTEN_BYTES_WIDTH, we need to seek a little less than a full row
                    ))?;
                }
            }
        }
        let encoded_row = F::raw_bytes_to_field_vec(&encoded_row_bytes);

        ensure!(encoded_row.len() == self.encoded_size);
        Ok(encoded_row)
    }

    pub fn replace_encoded_row(
        &mut self,
        target_row: usize,
        encoded_row_to_write: &[F],
    ) -> Result<()> {
        ensure!(
            target_row <= self.rows_written,
            "target row index is out of bounds"
        );
        assert_eq!(
            encoded_row_to_write.len(),
            self.encoded_size,
            "row is insufficient in size"
        );

        let row_bytes: Vec<u8> = F::field_vec_to_raw_bytes(encoded_row_to_write);
        assert_eq!(
            row_bytes.len(),
            self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize,
            "wrong number of bytes to write to file"
        );

        let column_length_in_bytes = self.row_capacity as i64 * F::WRITTEN_BYTES_WIDTH as i64;
        #[cfg(not(unix))]
        {
            let mut file_pointer = target_row * F::WRITTEN_BYTES_WIDTH as usize;
            self.file_to_read
                .seek(SeekFrom::Start(file_pointer as u64))?;
            let bytes_to_write_iterator = row_bytes.chunks(F::WRITTEN_BYTES_WIDTH as usize);

            let mut bytes_written = 0;
            for bytes_of_field_element in bytes_to_write_iterator.into_iter() {
                self.file_to_read.write_all(bytes_of_field_element)?;
                self.file_to_read.flush()?;
                self.file_to_read.seek(SeekFrom::Current(
                    column_length_in_bytes - F::WRITTEN_BYTES_WIDTH as i64,
                ))?;
                bytes_written += bytes_of_field_element.len();
            }
            ensure!(bytes_written == self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize);
        }
        #[cfg(unix)]
        {
            let file_pointer = target_row * F::WRITTEN_BYTES_WIDTH as usize;
            row_bytes
                .par_chunks(F::WRITTEN_BYTES_WIDTH as usize)
                .enumerate()
                .try_for_each(|(index, bytes_of_field_element)| -> Result<()> {
                    self.file_to_read.write_all_at(
                        bytes_of_field_element,
                        file_pointer as u64 + (index as u64 * column_length_in_bytes as u64),
                    )?;
                    Ok(())
                })?;
        }

        if target_row == self.rows_written {
            self.rows_written += 1;
        }
        Ok(())
    }

    pub fn get_encoded_column_without_path(&self, target_col: usize) -> Result<Vec<F>> {
        let start_byte = target_col * self.row_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let mut column_bytes: Vec<u8> =
            vec![0u8; self.rows_written * F::WRITTEN_BYTES_WIDTH as usize];
        self.file_to_read
            .read_exact_at(&mut column_bytes, start_byte as u64)?;

        let column: Vec<F> = F::raw_bytes_to_field_vec(&column_bytes);
        Ok(column)
    }

    pub fn process_file_to_merkle_tree(&mut self) -> Result<MerkleTree<D>> {
        // let mut column_digests: Vec<Output<D>> = Vec::with_capacity(self.encoded_size);
        let column_digests = (0..self.encoded_size)
            .into_par_iter()
            .map(|i| -> Result<Output<D>> {
                // column hashes start with a block of 0's
                let mut digest = D::new();
                Digest::update(&mut digest, <Output<D> as Default>::default());

                let column = self.get_encoded_column_without_path(i)?;
                for element in column {
                    element.digest_update(&mut digest);
                }
                Ok(digest.finalize())
            })
            .collect::<Result<Vec<_>>>()?;

        MerkleTree::new(&column_digests)
    }

    pub fn set_new_capacity(&mut self, new_row_capacity: usize) -> Result<()> {
        ensure!(
            new_row_capacity >= self.rows_written,
            "Cannot set capacity to fewer than rows are written. That would be destructive!"
        );

        self.file_to_read.set_len(
            new_row_capacity as u64 * self.encoded_size as u64 * F::WRITTEN_BYTES_WIDTH as u64,
        )?;

        // optimization: right now I'm gonna do it single threaded and in place but in the future it could be not
        //  in-place and multi-threaded. However, we don't need to optimize that for the paper :D

        let old_column_length = self.row_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let new_column_length = new_row_capacity * F::WRITTEN_BYTES_WIDTH as usize;
        let mut column_buffer = vec![0u8; new_column_length];

        for row in (0..self.encoded_size).rev() {
            // pick up old rows starting from the last one
            let old_row_start = row * old_column_length;
            // let bytes_read = self
            self.file_to_read
                .read_exact_at(&mut column_buffer[0..old_column_length], old_row_start as _)?;
            // column_buffer[bytes_read..old_column_length].fill(0);

            // place them in their final destination and flush out an entire row
            let new_row_start = row * new_column_length;
            self.file_to_read
                .write_at(&column_buffer, new_row_start as _)?;
        }

        self.row_capacity = new_row_capacity;
        Ok(())
    }
}

pub fn get_encoded_file_size_from_rate<F: DataField>(
    decoded_file_size: usize,
    pre_encoded_len: usize,
    encoded_len: usize,
) -> usize {
    // do not change order of operations, div ceils first and in *this* order is correct
    decoded_file_size
        .div_ceil(F::DATA_BYTE_CAPACITY as usize)
        .div_ceil(pre_encoded_len)
        * F::WRITTEN_BYTES_WIDTH as usize
        * encoded_len
}

pub fn get_decoded_file_size_from_rate<F: DataField>(
    encoded_file_size: usize,
    pre_encoded_len: usize,
    encoded_len: usize,
) -> usize {
    encoded_file_size
        .div_ceil(encoded_len)
        .div_ceil(F::WRITTEN_BYTES_WIDTH as usize)
        * F::DATA_BYTE_CAPACITY as usize
        * pre_encoded_len
}
