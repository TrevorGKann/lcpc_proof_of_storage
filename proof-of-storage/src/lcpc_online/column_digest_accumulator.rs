use crate::fields::data_field::DataField;
use crate::lcpc_online::merkle_tree::MerkleTree;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::FieldHash;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::marker::PhantomData;

#[derive(Debug, PartialEq)]
pub enum ColumnsToCareAbout {
    All,
    Only(Vec<usize>),
    // there are no guarantees on what happens if the columns to care about are not unique.
    // todo: make Only() take a reference to a [usize] instead of an owned vec
}

pub struct ColumnDigestAccumulator<D: Digest + FixedOutputReset + Send, F: DataField> {
    column_digests: Vec<D>,
    columns_to_care_about: ColumnsToCareAbout,
    data_field: PhantomData<F>,
}

impl<D: Digest + FixedOutputReset + Send, F: DataField> ColumnDigestAccumulator<D, F> {
    pub fn new(
        number_of_encoded_columns: usize,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Self {
        match columns_to_care_about {
            ColumnsToCareAbout::All => {
                let mut column_digests = Vec::with_capacity(number_of_encoded_columns);
                for _column in 0..number_of_encoded_columns {
                    let mut digest = D::new();
                    Digest::update(&mut digest, <Output<D> as Default>::default());
                    column_digests.push(digest);
                }
                ColumnDigestAccumulator {
                    column_digests,
                    columns_to_care_about,
                    data_field: PhantomData,
                }
            }
            ColumnsToCareAbout::Only(ref indices) => {
                let mut column_digests = Vec::with_capacity(indices.len());
                for _column in 0..indices.len() {
                    let mut digest = D::new();
                    Digest::update(&mut digest, <Output<D> as Default>::default());
                    column_digests.push(digest);
                }
                ColumnDigestAccumulator {
                    column_digests,
                    columns_to_care_about,
                    data_field: PhantomData,
                }
            }
        }
    }

    pub fn get_width(&self) -> usize {
        self.column_digests.len()
    }

    pub fn update(&mut self, encoded_row: &[F]) -> Result<()> {
        ensure!(
            encoded_row.len() == self.column_digests.len(),
            "incorrect length of input"
        ); // error: this is not suitable for if columns_to_care_about != all

        match self.columns_to_care_about {
            ColumnsToCareAbout::All => {
                (&mut self.column_digests, encoded_row)
                    .into_par_iter()
                    .for_each(|(digest, element)| element.digest_update(digest));
                // for (digest, input) in self.column_digests.iter_mut().zip(encoded_row) {
                //     input.digest_update(digest);
                // }
                // optimization: if digest is `send` then this can be a parfor and much faster
            }
            ColumnsToCareAbout::Only(ref columns) => {
                for columns_index in columns.iter() {
                    encoded_row[columns_index.clone()]
                        .digest_update(&mut self.column_digests[*columns_index]);
                }
            }
        }

        Ok(())
    }

    pub fn get_column_digests(self) -> Vec<Output<D>> {
        let mut hashes = Vec::with_capacity(self.column_digests.len());
        for digest in self.column_digests {
            hashes.push(digest.finalize());
        }

        hashes
    }

    /// Only works if ColumnsToCareAbout is All
    pub fn finalize_to_commit(self) -> Result<Output<D>> {
        ensure!(
            self.columns_to_care_about == ColumnsToCareAbout::All,
            "cannot commit to a root if not all columns have been tracked"
        );

        let tree = self.finalize_to_merkle_tree()?;
        Ok(tree.root())
    }

    pub fn finalize_to_merkle_tree(self) -> Result<MerkleTree<D>> {
        ensure!(
            self.columns_to_care_about == ColumnsToCareAbout::All,
            "cannot commit to a tree if not all columns have been tracked"
        );

        let leaves: Vec<Output<D>> = self.get_column_digests();
        MerkleTree::new(&leaves)
    }
}
