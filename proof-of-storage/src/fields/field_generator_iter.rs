use std::iter::Iterator;

use crate::fields::data_field::DataField;

pub struct FieldGeneratorIter<I, F>
where
    I: Iterator<Item = u8>,
    F: DataField,
{
    inner: I,
    buffer: F::DataBytes,
    buffer_pos: usize,
}

impl<I, F> FieldGeneratorIter<I, F>
where
    I: Iterator<Item = u8>,
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
    I: Iterator<Item = u8>,
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
    use std::io::{BufReader, Read};

    use crate::fields::field_generator_iter::FieldGeneratorIter;
    use crate::fields::{
        convert_byte_vec_to_field_elements_vec, read_file_path_to_field_elements_vec, WriteableFt63,
    };

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
    fn compare_iterator_on_a_file() {
        let test_file_path = "test_files/10000_byte_file.bytes";
        let test_file = std::fs::File::open(test_file_path).unwrap();

        let mut file_bytes = BufReader::new(test_file)
            .bytes()
            .map(|result| result.unwrap());

        let iterated_field_elements =
            FieldGeneratorIter::new(&mut file_bytes).collect::<Vec<WriteableFt63>>();

        let reference_field_elements = read_file_path_to_field_elements_vec(test_file_path);

        assert_eq!(iterated_field_elements, reference_field_elements);
    }
}
