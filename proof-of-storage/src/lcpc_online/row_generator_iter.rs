use std::iter::Iterator;

use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;

use crate::fields::data_field::DataField;

pub struct RowGeneratorIter<F, I, E, const num_pre: usize, const num_enc: usize>
where
    F: DataField,
    I: Iterator<Item=F>,
    E: LcEncoding,
{
    field_iterator: I,
    coefs_buffer: [F; num_enc], // fft done in place
    coef_buffer_position: usize,
    encoding: E,
}

impl<F, I, E, const num_pre: usize, const num_enc: usize> RowGeneratorIter<F, I, E, num_pre, num_enc>
where
    F: DataField,
    I: Iterator<Item=F>,
    E: LcEncoding,
{
    pub fn new(field_iterator: I) -> Self {
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
    E: LcEncoding,
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


        todo!()
    }
}

