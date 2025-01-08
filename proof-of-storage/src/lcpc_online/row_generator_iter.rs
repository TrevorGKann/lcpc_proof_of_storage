use std::iter::Iterator;

use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;

use crate::fields::data_field::DataField;

pub struct RowGeneratorIter<F, I, E, const num_pre: usize, const num_enc: usize>
where
    F: DataField,
    I: Iterator<Item=F>,
    E: LcEncoding<F=F>,
{
    field_iterator: I,
    coefs_buffer: [F; num_enc], // fft done in place
    coef_buffer_position: usize,
    encoding: E,
}

impl<F, I, const num_pre: usize, const num_enc: usize> RowGeneratorIter<F, I, LigeroEncoding<F>, num_pre, num_enc>
where
    F: DataField,
    I: Iterator<Item=F>,
    // E: LcEncoding<F=F>,
{
    pub fn new_ligero(field_iterator: I) -> Self {
        // let coefs_buffer = vec![F::ZERO; num_coefs];
        // let FFT_buffer = vec![F::ZERO; num_encoded_coefs];
        // let coef_buffer_position = 0;

        RowGeneratorIter {
            field_iterator,
            coefs_buffer: [F::ZERO; num_enc],
            coef_buffer_position: 0,
            encoding: LigeroEncoding::new_from_dims(num_pre, num_enc),
        }
    }
}

impl<F, I, E, const num_pre: usize, const num_enc: usize> Iterator for RowGeneratorIter<F, I, E,
    num_pre, num_enc>
where
    F: DataField,
    I: Iterator<Item=F>,
    E: LcEncoding<F=F>,
{
    type Item = [F; num_enc];

    fn next(&mut self) -> Option<Self::Item> {
        while self.coef_buffer_position < num_pre {
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
        self.coefs_buffer = [F::ZERO; num_enc];
        self.coef_buffer_position = 0;

        Some(return_val)
    }
}



#[cfg(test)]
mod tests {
    use rand::Rng;

    use crate::fields::{convert_byte_vec_to_field_elements_vec, WriteableFt63};
    use crate::fields::field_generator_iter::FieldGeneratorIter;
    use crate::lcpc_online::{convert_file_data_to_commit, CommitDimensions, CommitOrLeavesOutput, CommitRequestType};
    use blake3::Hasher as Blake3;
    use blake3::traits::digest::{Digest, Output};
    use futures::stream::iter;
    use lcpc_2d::{check_comm, LcCommit, ProverError};
    use lcpc_ligero_pc::LigeroEncoding;
    use crate::lcpc_online::row_generator_iter::RowGeneratorIter;

    #[test]
    fn is_row_iterator_the_same_as_non_iter() {

        static LEN: usize = 999;
        let mut bytes = vec![0u8; LEN];
        // fill bytes with random data
        let mut rng = rand::thread_rng();
        for byte in bytes.iter_mut() {
            *byte = rng.gen::<u8>();
        }

        const unencoded_len: usize = 4;
        const encoded_len: usize = 8;

        println!("making original style commit");
        let mut regular_commit: LcCommit<Blake3, LigeroEncoding<WriteableFt63>> = {
            let field_elements = convert_byte_vec_to_field_elements_vec(&bytes);
            let commit_result = convert_file_data_to_commit::<Blake3, WriteableFt63>(
                &field_elements,
                CommitRequestType::Commit,
                CommitDimensions::Specified { num_pre_encoded_columns: unencoded_len, num_encoded_columns: encoded_len }
            ).unwrap();
            let CommitOrLeavesOutput::Commit(commit) = commit_result else {panic!()};
            commit
        };

        println!("making iterated commit");
        let iterated_commit_rows = {
            let iter_field: FieldGeneratorIter<_, WriteableFt63> = FieldGeneratorIter::new(bytes.clone().into_iter());

            let iter_row: RowGeneratorIter<WriteableFt63, _, _,unencoded_len, encoded_len> = RowGeneratorIter::new_ligero(iter_field);

            iter_row
        };

        iterated_commit_rows
            .zip(regular_commit.comm.chunks(encoded_len))
            .for_each(|(iterated, regular)| {
                assert_eq!(iterated[..], *regular);
            });
    }
}

