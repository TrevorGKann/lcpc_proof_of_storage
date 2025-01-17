// use digest::{Digest, FixedOutputReset, Output};
use crate::fields::data_field::DataField;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use anyhow::Context;
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{merkle_tree, FieldHash, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;
use std::io::Read;
use std::iter::Iterator;

#[derive(Clone)]
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
        let mut digests: ColumnDigestAccumulator<D, F> =
            ColumnDigestAccumulator::new(self.coefs_buffer.len(), ColumnsToCareAbout::All);

        while let Some(row) = self.next() {
            digests.update(&row).unwrap() // should never panic because the size is fixed
        }

        digests.get_column_digests()
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
            for (digest_index, column_index) in column_indices.iter().enumerate() {
                row[*column_index].digest_update(&mut digests[digest_index]);
            }
        }

        let mut hashes = Vec::with_capacity(digests.len());
        for digest in digests {
            hashes.push(digest.finalize());
        }

        hashes
    }

    pub fn convert_to_commit_root<D>(self) -> Output<D>
    where
        D: Digest + FixedOutputReset,
    {
        let leaves: Vec<Output<D>> = self.get_column_digests::<D>();

        let mut nodes_of_tree: Vec<Output<D>> = vec![Output::<D>::default(); leaves.len() - 1];
        merkle_tree::<D>(&leaves, &mut nodes_of_tree);
        nodes_of_tree.last().unwrap().to_owned()
    }
}

impl<F, I> RowGeneratorIter<F, I, LigeroEncoding<F>>
where
    F: DataField,
    I: Iterator<Item = F>,
    // E = LigeroEncoding<F>
{
    pub fn new_ligero(field_iterator: I, num_pre_encoded: usize, num_encoded: usize) -> Self {
        let encoding = LigeroEncoding::new_from_dims(num_pre_encoded, num_encoded);

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

        // empty case
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
    use std::io::Read;

    use crate::fields::field_generator_iter::FieldGeneratorIter;
    use crate::fields::{
        convert_byte_vec_to_field_elements_vec, read_file_path_to_field_elements_vec,
        RandomBytesIterator, WriteableFt63,
    };
    use crate::lcpc_online::row_generator_iter::RowGeneratorIter;
    use crate::lcpc_online::{
        convert_file_data_to_commit, CommitDimensions, CommitOrLeavesOutput, CommitRequestType,
    };
    use blake3::traits::digest::Digest;
    use blake3::Hasher as Blake3;
    use itertools::Itertools;
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
    fn are_specified_columns_correct() {
        static LEN: usize = 999;
        let mut bytes = vec![0u8; LEN];
        // fill bytes with random data
        let mut rng = rand::thread_rng();
        for byte in bytes.iter_mut() {
            *byte = rng.gen::<u8>();
        }

        const UNENCODED_LEN: usize = 4;
        const ENCODED_LEN: usize = 8;

        println!("making iterated commit");
        let iterated_commit_rows = {
            let iter_field: FieldGeneratorIter<_, WriteableFt63> =
                FieldGeneratorIter::new(bytes.clone().into_iter());

            let iter_row: RowGeneratorIter<WriteableFt63, _, _> =
                RowGeneratorIter::new_ligero(iter_field, UNENCODED_LEN, ENCODED_LEN);

            iter_row
        };
        let iterated_digests = iterated_commit_rows.get_column_digests::<Blake3>();

        println!("making iterated commit with select columns");
        let random_indices = RandomBytesIterator::new()
            .take(4)
            .map(|i| i as usize % ENCODED_LEN)
            .unique()
            .collect::<Vec<usize>>();
        let partial_digests = {
            let iter_field: FieldGeneratorIter<_, WriteableFt63> =
                FieldGeneratorIter::new(bytes.clone().into_iter());

            let iter_row_partial: RowGeneratorIter<WriteableFt63, _, _> =
                RowGeneratorIter::new_ligero(iter_field, UNENCODED_LEN, ENCODED_LEN);

            iter_row_partial.get_specified_column_digests::<Blake3>(&random_indices)
        };

        for (partial_index, full_index) in random_indices.iter().enumerate() {
            assert_eq!(
                iterated_digests[*full_index],
                partial_digests[partial_index]
            );
        }
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

            iter_row.convert_to_commit_root::<Blake3>()
        };

        assert_eq!(regular_commit.get_root().root, iterated_commit_root)
    }

    #[test]
    fn read_file_to_root_with_iterator() {
        let test_file_path = "test_files/10000_byte_file.bytes";
        let file_field: Vec<WriteableFt63> = read_file_path_to_field_elements_vec(test_file_path);
        let CommitOrLeavesOutput::Commit(reference_commit) =
            convert_file_data_to_commit::<Blake3, _>(
                &file_field,
                CommitRequestType::Commit,
                CommitDimensions::Square,
            )
            .unwrap()
        else {
            panic!("didn't get commit!")
        };

        let buf_reader = std::io::BufReader::new(std::fs::File::open(&test_file_path).unwrap())
            .bytes()
            .map(|b| b.unwrap());
        let field_iterator = FieldGeneratorIter::<_, WriteableFt63>::new(buf_reader);
        let row_iterator = RowGeneratorIter::new_ligero(
            field_iterator,
            reference_commit.n_per_row,
            reference_commit.n_cols,
        );
        let streamed_root = row_iterator.convert_to_commit_root::<Blake3>();

        println!("reference root: {:?}", reference_commit.get_root().root);
        println!("streamed root: {:?}", streamed_root);
        assert_eq!(reference_commit.get_root().root, streamed_root);
    }
}
