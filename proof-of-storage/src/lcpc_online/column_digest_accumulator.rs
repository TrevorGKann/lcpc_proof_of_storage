use crate::fields::data_field::DataField;
use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{merkle_tree, FieldHash};
use std::marker::PhantomData;

pub struct ColumnDigestAccumulator<D: Digest + FixedOutputReset, F: DataField> {
    column_digests: Vec<D>,
    data_field: PhantomData<F>,
}

impl<D: Digest + FixedOutputReset, F: DataField> ColumnDigestAccumulator<D, F> {
    pub fn new(number_of_columns: usize) -> Self {
        let mut column_digests = Vec::with_capacity(number_of_columns);
        for column in 0..number_of_columns {
            let mut digest = D::new();
            Digest::update(&mut digest, <Output<D> as Default>::default());
            column_digests.push(digest);
        }
        ColumnDigestAccumulator {
            column_digests,
            data_field: PhantomData,
        }
    }

    pub fn get_width(&self) -> usize {
        self.column_digests.len()
    }

    pub fn update(&mut self, input: Vec<F>) -> Result<()> {
        ensure!(
            input.len() == self.column_digests.len(),
            "incorrect length of input"
        );

        for (mut digest, input) in self.column_digests.iter_mut().zip(input) {
            input.digest_update(digest);
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

    pub fn finalize_to_commit(self) -> Result<Output<D>> {
        let leaves: Vec<Output<D>> = self.get_column_digests();

        let mut nodes_of_tree: Vec<Output<D>> = vec![Output::<D>::default(); leaves.len() - 1];
        merkle_tree::<D>(&leaves, &mut nodes_of_tree);
        Ok(nodes_of_tree.last().unwrap().to_owned())
    }
}
