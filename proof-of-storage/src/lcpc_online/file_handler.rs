use crate::fields::data_field::DataField;
use crate::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use anyhow::Result;
use blake3::traits::digest::{Digest, Output};
use lcpc_2d::{LcColumn, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::marker::PhantomData;
use tokio::fs::File;
use ulid::Ulid;

enum CurrentState {
    Initialized,
    Verified,
    OutOfSync,
}

pub struct FileHandler<D: Digest, F: DataField, E: LcEncoding<F = F>> {
    file_ulid: Ulid,
    pre_encoded_size: usize,
    encoded_size: usize,
    num_rows: usize,
    total_bytes: usize,
    encoded_file_handle: File,
    unencoded_file_handle: File,
    merkle_tree_file_handle: File,

    _digest: PhantomData<D>,
    _field: PhantomData<F>,
    _encoding: PhantomData<E>,
}

impl<D: Digest, F: DataField> FileHandler<D, F, LigeroEncoding<F>> {
    pub fn new(ulid: Ulid) -> Result<Self> {
        todo!()
    }

    // returns the unencoded bytes that were edited as well as a FileHandler to the new version of itself
    // since edits need to be verified by the client before they can be confirmed.
    pub fn edit_bytes(
        &mut self,
        byte_start: usize,
        bytes_to_add: Vec<u8>,
    ) -> Result<Self, Vec<u8>> {
        todo!()
    }

    pub fn append_bytes(&mut self, bytes_to_add: Vec<u8>) -> Result<Self, Vec<u8>> {
        todo!()
    }

    pub fn get_unencoded_bytes(&mut self, byte_start: usize, byte_end: usize) -> Result<Vec<u8>> {
        todo!()
    }
}

impl<D: Digest, F: DataField, E: LcEncoding<F = F>> FileHandler<D, F, E> {
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

    pub fn get_merkle_tree(&mut self) -> Result<Output<D>> {
        todo!()
    }

    pub async fn recalculate_merkle_file(&mut self) -> Result<()> {
        todo!();
        Ok(())
    }

    pub async fn reencode_unencoded_file(&mut self) -> Result<()> {
        todo!();
        self.recalculate_merkle_file();
        Ok(())
    }

    async fn internal_get_merkle_path_for_column(
        &mut self,
        column_index: usize,
    ) -> Result<Vec<Output<D>>> {
        todo!()
    }

    async fn internal_get_encoded_column_without_path(
        &mut self,
        column_index: usize,
    ) -> Result<Vec<F>> {
        todo!()
    }

    async fn internal_open_column(&mut self, column_index: usize) -> Result<LcColumn<D, E>> {
        Ok(LcColumn::<D, E> {
            col: self
                .internal_get_encoded_column_without_path(column_index)
                .await?,
            path: self
                .internal_get_merkle_path_for_column(column_index)
                .await?,
        })
    }
}
