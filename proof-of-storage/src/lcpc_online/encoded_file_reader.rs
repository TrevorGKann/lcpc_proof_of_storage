use crate::fields::data_field::DataField;
use crate::lcpc_online::decode_row;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use crate::lcpc_online::merkle_tree::MerkleTree;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{FieldHash, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::cmp::{max, min};
use std::io::SeekFrom;
use std::iter::repeat_with;
use std::marker::PhantomData;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct EncodedFileReader<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>> {
    encoding: E,
    total_file_size: usize,
    file_to_read: File,
    pre_encoded_size: usize,
    encoded_size: usize,
    num_rows: usize,
    merkle_paths: Vec<Output<D>>,
    _field_type: PhantomData<F>,
    _digest_type: PhantomData<D>,
}

impl<F: DataField, D: Digest + FixedOutputReset> EncodedFileReader<F, D, LigeroEncoding<F>> {
    pub async fn new_ligero(
        file_to_read: File,
        pre_encoded_size: usize,
        encoded_size: usize,
        // num_rows: usize,
    ) -> Self {
        let encoding = LigeroEncoding::<F>::new_from_dims(pre_encoded_size, encoded_size);
        let total_file_size = file_to_read.metadata().await.unwrap().len() as usize;
        let num_rows = total_file_size.div_ceil(encoded_size * F::WRITTEN_BYTES_WIDTH as usize);

        Self {
            encoding,
            total_file_size,
            file_to_read,
            pre_encoded_size,
            encoded_size,
            num_rows,
            merkle_paths: Vec::new(),
            _field_type: PhantomData,
            _digest_type: PhantomData,
        }
    }

    pub async fn get_unencoded_row(&mut self, target_row: usize) -> Result<Vec<F>> {
        ensure!(
            target_row < self.num_rows,
            "target row index is out of bounds"
        );

        let encoded_row = self.get_encoded_row(target_row).await?;

        let decoded_row = decode_row(encoded_row)?;

        Ok(decoded_row)
    }

    pub async fn get_unencoded_row_bytes(&mut self, target_row: usize) -> Result<Vec<u8>> {
        Ok(F::field_vec_to_raw_bytes(
            &self.get_unencoded_row(target_row).await?,
        ))
    }

    pub async fn decode_to_target_file(&mut self, target_file: &mut File) -> Result<()> {
        for row_i in 0..self.num_rows {
            target_file
                .write_all(self.get_unencoded_row_bytes(row_i).await?.as_slice())
                .await?;
        }
        target_file.flush().await?;
        Ok(())
    }

    pub async fn get_unencoded_file_len(&self) -> Result<usize> {
        Ok(self.file_to_read.metadata().await?.len() as usize
            / (self.encoded_size / self.pre_encoded_size))
    }

    pub async fn resize_to_target_file(
        &mut self,
        target_file: File,
        new_pre_encoded_size: usize,
        new_encoded_size: usize,
    ) -> Result<Vec<Output<D>>> {
        let mut new_encoded_file_writer = EncodedFileWriter::<F, D, LigeroEncoding<F>>::new(
            new_pre_encoded_size,
            new_encoded_size,
            self.get_unencoded_file_len().await?,
            target_file,
        );

        for row_i in 0..self.num_rows {
            new_encoded_file_writer
                .push_bytes(self.get_unencoded_row_bytes(row_i).await?.as_slice())
                .await?;
        }

        new_encoded_file_writer.finalize_to_column_digest().await
    }

    /// returns the original data that was replaced in the unencoded file
    pub async fn edit_row(
        &mut self,
        unencoded_start_byte: usize,
        new_unencoded_data: &[u8],
    ) -> Result<Vec<u8>> {
        let unencoded_start_field_element = unencoded_start_byte / F::DATA_BYTE_CAPACITY as usize;
        let unencoded_end_field_element =
            (unencoded_start_byte + new_unencoded_data.len()) / F::DATA_BYTE_CAPACITY as usize;

        let start_row = unencoded_start_field_element / self.pre_encoded_size;
        let end_row = unencoded_end_field_element / self.pre_encoded_size;

        let mut original_unencoded_bytes_result: Vec<u8> =
            Vec::with_capacity(new_unencoded_data.len());
        let mut current_byte = unencoded_start_byte;
        let end_byte = current_byte + new_unencoded_data.len();
        let mut bytes_written = 0;
        for row_index in start_row..=end_row {
            let mut original_bytes = self.get_unencoded_row_bytes(row_index).await?;

            let start_of_row_byte_index =
                row_index * self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize;
            let end_of_row_byte_index =
                (row_index + 1) * self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize - 1;

            let start_of_bytes_to_care_for = max(current_byte, start_of_row_byte_index)
                % (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
            let end_of_bytes_to_care_for = min(end_byte, end_of_row_byte_index)
                % (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);

            original_unencoded_bytes_result.extend_from_slice(
                &original_bytes[start_of_bytes_to_care_for..end_of_bytes_to_care_for],
            );

            for byte_index in start_of_bytes_to_care_for..end_of_bytes_to_care_for {
                original_bytes[byte_index] = new_unencoded_data[bytes_written];
                bytes_written += 1;
            }

            let mut new_row = F::from_byte_vec(&original_bytes);
            new_row.extend(repeat_with(|| F::ZERO).take(self.encoded_size - new_row.len()));
            self.encoding.encode(&mut new_row)?;
            self.replace_encoded_row(row_index, new_row).await?
        }

        Ok(original_unencoded_bytes_result)
    }
}

// generic over all encodings
impl<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>> EncodedFileReader<F, D, E> {
    pub async fn get_encoded_row(&mut self, target_row: usize) -> Result<Vec<F>> {
        // let mut row_start_byte = target_row * self.num_rows * F::WRITTEN_BYTES_WIDTH as usize;
        let mut row_start_byte = target_row * F::WRITTEN_BYTES_WIDTH as usize;
        let mut encoded_row_bytes: Vec<u8> =
            vec![0; self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize];

        self.file_to_read
            .seek(SeekFrom::Start(row_start_byte as u64))
            .await?;

        self.file_to_read.read_exact(&mut encoded_row_bytes).await?;

        Ok(F::raw_bytes_to_field_vec(&encoded_row_bytes))
    }

    pub async fn replace_encoded_row(
        &mut self,
        target_row: usize,
        data_to_write: Vec<F>,
    ) -> Result<()> {
        ensure!(
            target_row < self.num_rows,
            "target row index is out of bounds"
        );
        ensure!(
            data_to_write.len() == self.encoded_size,
            "row is insufficient in size"
        );

        let start_byte = target_row * self.num_rows * F::WRITTEN_BYTES_WIDTH as usize;
        self.file_to_read
            .seek(SeekFrom::Start(start_byte as u64))
            .await?;

        for element in data_to_write {
            self.file_to_read
                .write_all(&F::to_data_bytes(&element).as_ref())
                .await?;
            self.file_to_read
                .seek(SeekFrom::Current(
                    (self.num_rows - 1) as i64 * F::WRITTEN_BYTES_WIDTH as i64,
                ))
                .await?;
        }

        Ok(())
    }

    pub async fn get_encoded_column_without_path(&mut self, target_col: usize) -> Result<Vec<F>> {
        self.file_to_read
            .seek(SeekFrom::Start(
                (target_col * self.num_rows) as u64 * F::WRITTEN_BYTES_WIDTH as u64,
            ))
            .await?;
        let mut column_bytes: Vec<u8> =
            vec![0u8; self.encoded_size * F::WRITTEN_BYTES_WIDTH as usize];
        self.file_to_read.read_exact(&mut column_bytes).await?;

        let column: Vec<F> = F::raw_bytes_to_field_vec(&column_bytes);
        Ok(column)
    }

    pub async fn process_file_to_merkle_tree(mut self) -> Result<MerkleTree<D>> {
        let mut column_digests: Vec<Output<D>> = Vec::with_capacity(self.encoded_size);
        for i in 0..self.encoded_size {
            // column hashes start with a block of 0's
            let mut digest = D::new();
            Digest::update(&mut digest, <Output<D> as Default>::default());

            let column = self.get_encoded_column_without_path(i).await?;
            for element in column {
                element.digest_update(&mut digest);
            }

            column_digests.push(digest.finalize());
        }

        MerkleTree::new(&column_digests)
    }
}
