use crate::fields::data_field::DataField;
use crate::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use crate::lcpc_online::decode_row;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{merkle_tree, FieldHash, LcColumn, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::io::SeekFrom;
use std::marker::PhantomData;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct UnProcessedState;
pub struct ProcessedState;
pub struct EditedState;

pub struct EncodedFileReader<
    F: DataField,
    D: Digest + FixedOutputReset,
    E: LcEncoding<F = F>,
    State,
> {
    encoding: E,
    total_file_size: usize,
    file_to_read: File,
    pre_encoded_size: usize,
    encoded_size: usize,
    num_rows: usize,
    column_digests: Vec<Output<D>>,
    merkle_paths: Vec<Output<D>>,
    _field_type: PhantomData<F>,
    _digest_type: PhantomData<D>,
    _state: PhantomData<State>,
}

impl<F: DataField, D: Digest + FixedOutputReset>
    EncodedFileReader<F, D, LigeroEncoding<F>, UnProcessedState>
{
    pub async fn new(
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
            column_digests: Vec::new(),
            merkle_paths: Vec::new(),
            _field_type: PhantomData,
            _digest_type: PhantomData,
            _state: PhantomData,
        }
    }
}

impl<F: DataField, D: Digest + FixedOutputReset, AnyState>
    EncodedFileReader<F, D, LigeroEncoding<F>, AnyState>
{
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
    ) -> Result<Output<D>> {
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

        new_encoded_file_writer.finalize_to_commit().await
    }

    /// returns an edited state as well as the original data that was replaced in the unencoded file
    pub async fn edit_row(
        mut self,
        unencoded_start_byte: usize,
        new_unencoded_data: Vec<u8>,
    ) -> Result<(EncodedFileReader<F, D, LigeroEncoding<F>, EditedState>, Vec<u8>)> {
        todo!()
    }
}

impl<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>, AnyState>
    EncodedFileReader<F, D, E, AnyState>
{
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
}
impl<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>>
    EncodedFileReader<F, D, E, UnProcessedState>
{
    pub async fn process_file(mut self) -> Result<EncodedFileReader<F, D, E, ProcessedState>> {
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

        Ok(EncodedFileReader {
            encoding: self.encoding,
            total_file_size: self.total_file_size,
            file_to_read: self.file_to_read,
            pre_encoded_size: self.pre_encoded_size,
            encoded_size: self.encoded_size,
            num_rows: self.num_rows,
            column_digests: column_digests,
            merkle_paths: path_digests,
            _field_type: PhantomData,
            _digest_type: PhantomData,
            _state: PhantomData,
        })
    }
}
impl<F: DataField, D: Digest + FixedOutputReset, E: LcEncoding<F = F>>
    EncodedFileReader<F, D, E, ProcessedState>
{
    // warning: This function can take up more memory that is available if it's attached to a large file
    pub async fn read_full_columns(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Vec<LcColumn<D, E>> {
        todo!()
    }

    pub async fn read_only_digests(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Vec<Output<D>> {
        todo!()
    }

    pub async fn read_to_commit(&mut self) -> Result<Output<D>> {
        Ok(self.get_merkle_tree().last().unwrap().to_owned())
    }

    pub fn get_merkle_path(&mut self, column_index: usize) -> Vec<Output<D>> {
        todo!()
    }

    pub fn get_merkle_tree(&self) -> Vec<Output<D>> {
        let mut merkle_tree: Vec<Output<D>> = Vec::with_capacity(self.encoded_size * 2 - 1);
        merkle_tree.extend_from_slice(&self.column_digests);
        merkle_tree.extend_from_slice(&self.merkle_paths);
        merkle_tree
    }

    pub async fn get_encoded_columns_with_paths(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Result<Vec<LcColumn<D, E>>> {
        match columns_to_care_about {
            ColumnsToCareAbout::All => {
                let mut opened_columns: Vec<LcColumn<D, E>> = Vec::with_capacity(self.encoded_size);
                for i in 0..self.encoded_size {
                    opened_columns.push(self.internal_open_column(i).await?)
                }
                Ok(opened_columns)
            }
            ColumnsToCareAbout::Only(column_indices) => {
                let mut opened_columns: Vec<LcColumn<D, E>> = Vec::with_capacity(self.encoded_size);
                for i in column_indices.iter() {
                    opened_columns.push(self.internal_open_column(*i).await?)
                }

                Ok(opened_columns)
            }
        }
    }

    async fn internal_open_column(&mut self, column_index: usize) -> Result<LcColumn<D, E>> {
        Ok(LcColumn::<D, E> {
            col: self.get_encoded_column_without_path(column_index).await?,
            path: self.get_merkle_path(column_index),
        })
    }
}

//////////////////////////////////////// todo: sort past here
////////////////////////////////////////  ?
////////////////////////////////////////    ?
////////////////////////////////////////      ?
