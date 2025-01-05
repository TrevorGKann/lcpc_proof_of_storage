use std::iter::Iterator;

use crate::fields::data_field::DataField;

pub struct FieldGeneratorIter<I, F>
where
    I: Iterator<Item=u8>,
    F: DataField,
{
    inner: I,
    buffer: F::DataBytes,
    buffer_pos: usize,
}

impl<I, F> FieldGeneratorIter<I, F>
where
    I: Iterator<Item=u8>,
    F: DataField,
{
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            buffer: F::DataBytes::default(),
            buffer_pos: 0,
        }
    }
}

impl<I, F> Iterator for FieldGeneratorIter<I, F>
where
    I: Iterator<Item=u8>,
    F: DataField,
{
    type Item = F;

    fn next(&mut self) -> Option<Self::Item> {
        while self.buffer_pos < F::DATA_BYTE_CAPACITY as usize {
            if let Some(byte) = self.inner.next() {
                self.buffer.as_mut()[self.buffer_pos] = byte;
                self.buffer_pos += 1;
            } else {
                break;
            }
        }


        if self.buffer_pos == 0 {
            return None;
        }

        let output = F::from_data_bytes(&self.buffer);
        self.buffer = F::DataBytes::default();
        self.buffer_pos = 0;

        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use rand::Rng;

    use crate::fields::{convert_byte_vec_to_field_elements_vec, WriteableFt63};
    use crate::fields::field_generator_iter::FieldGeneratorIter;

    #[test]
    fn compare_iterator_to_normal() {
        static LEN: usize = 999;
        let mut bytes = vec![0u8; LEN];
        // fill bytes with random data
        let mut rng = rand::thread_rng();
        for byte in bytes.iter_mut() {
            *byte = rng.gen::<u8>();
        }
        let field = convert_byte_vec_to_field_elements_vec::<WriteableFt63>(&bytes.clone());
        let iter_field: Vec<WriteableFt63> = FieldGeneratorIter::new(bytes.into_iter()).collect();


        assert_eq!(iter_field, field);
    }

    #[test]
    fn compare_iterator_on_a_file() {}
}