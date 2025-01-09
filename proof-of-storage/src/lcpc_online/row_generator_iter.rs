// use digest::{Digest, FixedOutputReset, Output};
use crate::fields::data_field::DataField;
use anyhow::{Context, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{merkle_tree, FieldHash, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::iter::Iterator;

pub struct RowGeneratorIter<F, I, E>
where
    F: DataField,
    I: Iterator<Item = F>,
    E: LcEncoding<F = F>,
{
    field_iterator: I,
    coefs_buffer: Vec<F>, // fft done in place
    coef_buffer_position: usize,
    unencoded_len: usize,
    encoded_len: usize,
    encoding: E,
}

impl<F, I, E> RowGeneratorIter<F, I, E>
where
    F: DataField,
    I: Iterator<Item = F>,
    E: LcEncoding<F = F>,
{
    pub fn get_column_digests<D>(mut self) -> Vec<Output<D>>
    where
        D: Digest + FixedOutputReset,
    {
        let mut digests = Vec::with_capacity(self.coefs_buffer.len());
        for _ in 0..self.coefs_buffer.len() {
            let mut digest = D::new();
            Digest::update(&mut digest, <Output<D> as Default>::default());
            digests.push(digest);
        }

        while let Some(row) = self.next() {
            for (column_index, digest) in digests.iter_mut().enumerate() {
                row[column_index].digest_update(digest);
            }
        }

        let mut hashes = Vec::with_capacity(digests.len());
        for digest in digests {
            hashes.push(digest.finalize());
        }

        hashes
    }

    pub fn get_specified_column_digests<D>(mut self, column_indices: &[usize]) -> Vec<Output<D>>
    where
        D: Digest + FixedOutputReset,
    {
        let mut digests = Vec::with_capacity(column_indices.len());
        for _ in 0..column_indices.len() {
            let mut digest = D::new();
            Digest::update(&mut digest, <Output<D> as Default>::default());
            digests.push(digest);
        }

        while let Some(row) = self.next() {
            for column_index in column_indices {
                row[*column_index].digest_update(&mut digests[*column_index]);
            }
        }

        let mut hashes = Vec::with_capacity(digests.len());
        for digest in digests {
            hashes.push(digest.finalize());
        }

        hashes
    }

    pub fn convert_to_commit_root<D>(self) -> Result<Output<D>>
    where
        D: Digest + FixedOutputReset,
    {
        let leaves: Vec<Output<D>> = self.get_column_digests::<D>();

        let len_of_merkle_tree = leaves
            .len()
            .checked_next_power_of_two()
            .context("no next power of two")?
            - 1;

        // step 2: compute rest of Merkle tree
        let mut nodes_of_tree: Vec<Output<D>> = vec![Output::<D>::default(); leaves.len() - 1];
        merkle_tree::<D>(&leaves, &mut nodes_of_tree);
        Ok(nodes_of_tree.last().unwrap().to_owned())
    }
}

impl<F, I> RowGeneratorIter<F, I, LigeroEncoding<F>>
where
    F: DataField,
    I: Iterator<Item = F>,
    // E = LigeroEncoding<F>
{
    pub fn new_ligero(field_iterator: I, num_pre_encoded: usize, num_encoded: usize) -> Self {
        // let coefs_buffer = vec![F::ZERO; num_coefs];
        // let FFT_buffer = vec![F::ZERO; num_encoded_coefs];
        // let coef_buffer_position = 0;

        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded, num_encoded);

        // todo: fixup with heap allocaiton
        RowGeneratorIter {
            field_iterator,
            coefs_buffer: vec![F::ZERO; num_encoded],
            coef_buffer_position: 0,
            unencoded_len: num_pre_encoded,
            encoded_len: num_encoded,
            encoding,
        }
    }
}

impl<F, I, E> Iterator for RowGeneratorIter<F, I, E>
where
    F: DataField,
    I: Iterator<Item = F>,
    E: LcEncoding<F = F>,
{
    type Item = Vec<F>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.coef_buffer_position < self.unencoded_len {
            if let Some(field_element) = self.field_iterator.next() {
                self.coefs_buffer[self.coef_buffer_position] = field_element;
                self.coef_buffer_position = self.coef_buffer_position + 1;
            } else {
                break;
            }
        }

        if self.coef_buffer_position == 0 {
            return None;
        }

        self.encoding.encode(&mut self.coefs_buffer).unwrap(); //todo: safe handle of error
        let return_val = self.coefs_buffer.clone();

        // reset internal values
        for item in self.coefs_buffer.iter_mut() {
            *item = F::ZERO;
        }
        self.coef_buffer_position = 0;

        Some(return_val)
    }
}

#[cfg(test)]
mod tests {
    use rand::Rng;

    use crate::fields::field_generator_iter::FieldGeneratorIter;
    use crate::fields::{convert_byte_vec_to_field_elements_vec, WriteableFt63};
    use crate::lcpc_online::row_generator_iter::RowGeneratorIter;
    use crate::lcpc_online::{
        convert_file_data_to_commit, CommitDimensions, CommitOrLeavesOutput, CommitRequestType,
    };
    use blake3::traits::digest::Digest;
    use blake3::Hasher as Blake3;
    use lcpc_2d::LcCommit;
    use lcpc_ligero_pc::LigeroEncoding;

    #[test]
    fn is_row_iterator_the_same_as_non_iter() {
        static LEN: usize = 999;
        let mut bytes = vec![0u8; LEN];
        // fill bytes with random data
        let mut rng = rand::thread_rng();
        for byte in bytes.iter_mut() {
            *byte = rng.gen::<u8>();
        }

        const UNENCODED_LEN: usize = 4;
        const ENCODED_LEN: usize = 8;

        println!("making original style commit");
        let mut regular_commit: LcCommit<Blake3, LigeroEncoding<WriteableFt63>> = {
            let field_elements = convert_byte_vec_to_field_elements_vec(&bytes);
            let commit_result = convert_file_data_to_commit::<Blake3, WriteableFt63>(
                &field_elements,
                CommitRequestType::Commit,
                CommitDimensions::Specified {
                    num_pre_encoded_columns: UNENCODED_LEN,
                    num_encoded_columns: ENCODED_LEN,
                },
            )
            .unwrap();
            let CommitOrLeavesOutput::Commit(commit) = commit_result else {
                panic!()
            };
            commit
        };

        println!("making iterated commit");
        let iterated_commit_rows = {
            let iter_field: FieldGeneratorIter<_, WriteableFt63> =
                FieldGeneratorIter::new(bytes.clone().into_iter());

            let iter_row: RowGeneratorIter<WriteableFt63, _, _> =
                RowGeneratorIter::new_ligero(iter_field, UNENCODED_LEN, ENCODED_LEN);

            iter_row
        };

        iterated_commit_rows
            .zip(regular_commit.comm.chunks(ENCODED_LEN))
            .for_each(|(iterated, regular)| {
                assert_eq!(iterated[..], *regular);
            });
    }

    #[test]
    fn is_row_iterator_the_same_root() {
        static LEN: usize = 999;
        let mut bytes = vec![0u8; LEN];
        // fill bytes with random data
        let mut rng = rand::thread_rng();
        for byte in bytes.iter_mut() {
            *byte = rng.gen::<u8>();
        }

        const UNENCODED_LEN: usize = 4;
        const ENCODED_LEN: usize = 8;

        println!("making original style commit");
        let mut regular_commit: LcCommit<Blake3, LigeroEncoding<WriteableFt63>> = {
            let field_elements = convert_byte_vec_to_field_elements_vec(&bytes);
            let commit_result = convert_file_data_to_commit::<Blake3, WriteableFt63>(
                &field_elements,
                CommitRequestType::Commit,
                CommitDimensions::Specified {
                    num_pre_encoded_columns: UNENCODED_LEN,
                    num_encoded_columns: ENCODED_LEN,
                },
            )
            .unwrap();
            let CommitOrLeavesOutput::Commit(commit) = commit_result else {
                panic!()
            };
            commit
        };

        println!("making iterated commit");
        let iterated_commit_root = {
            let iter_field: FieldGeneratorIter<_, WriteableFt63> =
                FieldGeneratorIter::new(bytes.clone().into_iter());

            let iter_row: RowGeneratorIter<WriteableFt63, _, _> =
                RowGeneratorIter::new_ligero(iter_field, UNENCODED_LEN, ENCODED_LEN);

            iter_row.convert_to_commit_root::<Blake3>().unwrap()
        };

        assert_eq!(regular_commit.get_root().root, iterated_commit_root)
    }
}
