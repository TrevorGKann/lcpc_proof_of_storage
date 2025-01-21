use crate::fields::data_field::DataField;
use crate::lcpc_online::decode_row;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{merkle_tree, FieldHash, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::io::SeekFrom;
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
        num_rows: usize,
    ) -> Self {
        let encoding = LigeroEncoding::<F>::new_from_dims(pre_encoded_size, encoded_size);
        let total_file_size = file_to_read.metadata().await.unwrap().len() as usize;

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

    pub async fn get_unencoded_row(&mut self, target_row: usize) -> Result<Vec<u8>> {
        ensure!(
            target_row < self.num_rows,
            "target row index is out of bounds"
        );

        let encoded_row = self.get_encoded_row(target_row).await;

        let decoded_row = decode_row(encoded_row)?;

        Ok(F::field_vec_to_byte_vec(&decoded_row))
    }

    pub async fn unencode_to_target_file(&mut self, target_file: &mut File) -> Result<()> {
        for row_i in 0..self.num_rows {
            target_file
                .write_all(self.get_unencoded_row(row_i).await?.as_slice())
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
                .push_bytes(self.get_unencoded_row(row_i).await?.as_slice())
                .await?;
        }

        Ok(new_encoded_file_writer.finalize_to_column_digest())
    }

    /// returns the original data that was replaced in the unencoded file
    async fn edit_row(
        mut self,
        unencoded_start_byte: usize,
        new_unencoded_data: Vec<u8>,
    ) -> Result<Vec<u8>> {
        todo!()
    }
}

// generic over all encodings
impl<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>> EncodedFileReader<F, D, E> {
    pub async fn get_encoded_row(&mut self, target_row: usize) -> Vec<F> {
        todo!()
    }
    pub async fn get_encoded_column_without_path(&mut self, target_col: usize) -> Result<Vec<F>> {
        self.file_to_read
            .seek(SeekFrom::Start(
                (target_col * self.num_rows) as u64 * F::DATA_BYTE_CAPACITY as u64,
            ))
            .await?;
        let mut column_bytes = vec![0u8; self.encoded_size * F::DATA_BYTE_CAPACITY as usize];
        self.file_to_read.read_exact(&mut column_bytes);

        Ok(F::from_byte_vec(&column_bytes))
    }

    pub async fn process_file_to_merkle_tree(mut self) -> Result<Vec<Output<D>>> {
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

        let mut path_digests: Vec<Output<D>> = Vec::with_capacity(self.encoded_size - 1);
        merkle_tree::<D>(&column_digests, &mut path_digests);

        todo!()
    }
}

//////////////////////////////////////// todo: sort past here
////////////////////////////////////////  ?
////////////////////////////////////////    ?
////////////////////////////////////////      ?

// warning: This function can take up more memory that is available if it's attached to a large file
