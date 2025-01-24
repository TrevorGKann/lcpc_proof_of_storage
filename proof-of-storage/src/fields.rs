use std::fs::File;
use std::io::{Read, Write};
use std::ops::Div;

use anyhow::Result;
use bitvec::view::BitViewSized;
use ff::{Field, PrimeField};
use itertools::Itertools;
use rand::Rng;
use tokio::io::{AsyncReadExt, BufReader};

use crate::fields::data_field::DataField;

pub mod data_field;
pub mod field_generator_iter;
mod ft253_192;
pub use ft253_192::Ft253_192;
mod random_byte_iterator;
pub use random_byte_iterator::RandomBytesIterator;
pub mod writable_ft63;
pub use writable_ft63::WriteableFt63;

#[derive(Debug)]
pub enum FieldErr {
    InvalidFieldElement,
}
#[tracing::instrument]
pub fn read_file_to_field_elements_vec<F: DataField>(file: &mut File) -> (usize, Vec<F>) {
    // todo: need to convert to async with tokio file
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    tracing::trace!("read {} bytes from file", buffer.len());

    (
        buffer.len(),
        convert_byte_vec_to_field_elements_vec(&buffer),
    )
}

#[tracing::instrument]
pub async fn stream_file_to_field_elements_vec<F: DataField>(
    file: &mut tokio::fs::File,
) -> Result<(usize, Vec<F>)> {
    // buf_size has to be a multiple of DATA_BYTE_CAPACITY since the buf reader will not cyclically
    //  fill but instead will fill a partial one at the end of the buffer.
    const BUF_MULT: usize = 1000;
    let buf_size: usize = BUF_MULT * F::DATA_BYTE_CAPACITY as usize;

    let file_size = file.metadata().await?.len();
    let total_elems = file_size.div_ceil(F::DATA_BYTE_CAPACITY as u64);
    let mut field_vec: Vec<F> = Vec::with_capacity(total_elems as usize);

    let mut reader = BufReader::with_capacity(buf_size, file);
    let mut buffer = F::DataBytes::default();

    loop {
        // Read bytes into the buffer
        let bytes_read = reader.read(buffer.as_mut()).await?;
        if bytes_read == 0 {
            break; // EOF reached
        }
        if bytes_read < F::DATA_BYTE_CAPACITY as usize {
            for i in bytes_read..F::DATA_BYTE_CAPACITY as usize {
                buffer.as_mut()[i] = 0u8;
            }
        }

        // Convert the buffer slice into field elements and extend the vector
        field_vec.push(F::from_data_bytes(&buffer));
    }

    Ok((file_size as usize, field_vec))
}

#[tracing::instrument]
pub fn stream_file_to_field_elements_vec_sync<F: DataField>(
    file: &mut std::fs::File,
) -> Result<(usize, Vec<F>)> {
    // BUF_SIZE has to be a multiple of DATA_BYTE_CAPACITY since the buf reader will not cyclically
    //  fill but instead will fill a partial one at the end of the buffer.
    const BUF_MULT: usize = 1000;
    let buf_size: usize = BUF_MULT * F::DATA_BYTE_CAPACITY as usize;

    let file_size = file.metadata()?.len();
    let total_elems = file_size.div_ceil(F::DATA_BYTE_CAPACITY as u64);
    let mut field_vec: Vec<F> = Vec::with_capacity(total_elems as usize);

    let mut reader = std::io::BufReader::with_capacity(buf_size, file);
    let mut buffer = F::DataBytes::default();

    loop {
        // Read bytes into the buffer
        let bytes_read = reader.read(buffer.as_mut())?;
        if bytes_read == 0 {
            break; // EOF reached
        }
        if bytes_read < F::DATA_BYTE_CAPACITY as usize {
            for i in bytes_read..F::DATA_BYTE_CAPACITY as usize {
                buffer.as_mut()[i] = 0u8;
            }
        }

        // Convert the buffer slice into field elements and extend the vector
        field_vec.push(F::from_data_bytes(&buffer));
    }

    Ok((file_size as usize, field_vec))
}

// #[tracing::instrument]
pub fn convert_byte_vec_to_field_elements_vec<F: DataField>(byte_vec: &[u8]) -> Vec<F> {
    // todo: can refactor this entire function as a F::from_byte_vec(byte_vec)
    F::from_byte_vec(byte_vec)
}

#[tracing::instrument]
pub fn convert_field_elements_vec_to_byte_vec<F: DataField>(
    field_elements: &[F],
    expected_length: usize,
) -> Vec<u8> {
    let mut bytes = F::field_vec_to_byte_vec(field_elements);
    bytes.truncate(expected_length);
    bytes
}

#[tracing::instrument]
pub fn read_file_path_to_field_elements_vec<F: DataField>(path: &str) -> Vec<F> {
    let mut file = File::open(path).unwrap();
    let (_, result) = read_file_to_field_elements_vec::<F>(&mut file);
    result
}

pub fn field_elements_vec_to_file<F: DataField>(path: &str, field_elements: &Vec<F>) {
    let mut file = File::create(path).unwrap();

    for (index, field_element) in field_elements.iter().enumerate() {
        let mut write_buffer = F::field_vec_to_byte_vec(&[*field_element]);

        //check if we are looking at the last `field_element` in `field_elements`
        //if so, we need to drop trailing zeroes as well
        if index == field_elements.len() - 1 {
            while write_buffer.last() == Some(&0) {
                write_buffer.pop();
            }
            file.write_all(&write_buffer).unwrap();
        } else {
            file.write_all(&write_buffer).unwrap();
        }
    }
}

pub fn random_writeable_field_vec<F: DataField>(log_len: usize) -> Vec<F> {
    use std::iter::repeat_with;

    let mut rng = rand::thread_rng();

    let read_in_bytes = F::DATA_BYTE_CAPACITY * (1 << log_len);
    let byte_vec = repeat_with(|| rng.gen::<u8>())
        .take(read_in_bytes as usize)
        .collect_vec();
    F::from_byte_vec(&byte_vec)
}

/// Note that the polynomial is evaluated little endian style, e.g. the array of coefficients should be passed in as
/// [c_0, c_1, ..., c_d] where c_i is the coefficient of x^i.
pub fn evaluate_field_polynomial_at_point<F: PrimeField>(field_elements: &Vec<F>, point: &F) -> F {
    let mut result = F::ZERO;
    let mut current_power = F::ONE;
    for field_element in field_elements {
        result += *field_element * current_power;
        current_power *= point;
    }
    result
}

// evaluates a polynomial collection as if it started at the nth degree. E.g. ([1,2,3],x,3) would be evaluated
//      as 1x^3 + 2x^4 + 3x^5
pub fn evaluate_field_polynomial_at_point_with_elevated_degree<F: PrimeField>(
    field_elements: &[F],
    point: &F,
    degree_offset: u64,
) -> F {
    let mut result = F::ZERO;
    let mut current_power = point.pow([degree_offset]);
    for field_element in field_elements.iter() {
        result += *field_element * current_power;
        current_power *= point;
    }
    result
}

pub fn vector_multiply<F: PrimeField>(a: &[F], b: &[F]) -> F {
    a.iter().zip(b.iter()).map(|(a, b)| *a * b).sum()
}

pub fn is_power_of_two(x: usize) -> bool {
    x & (x - 1) == 0
}

#[test]
fn test_polynomial_eval() {
    let field_elements = vec![
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
    ];
    let point = WriteableFt63::ONE;
    assert_eq!(
        evaluate_field_polynomial_at_point(&field_elements, &point),
        WriteableFt63::ZERO
    );

    let field_elements = vec![WriteableFt63::ONE, WriteableFt63::ONE, WriteableFt63::ONE];
    assert_eq!(
        evaluate_field_polynomial_at_point(&field_elements, &point),
        WriteableFt63::ONE + WriteableFt63::ONE + WriteableFt63::ONE
    );

    let field_elements = vec![
        WriteableFt63::ZERO,
        WriteableFt63::from_u128(1),
        WriteableFt63::from_u128(2),
    ];
    let point = WriteableFt63::from_u128(2);
    assert_eq!(
        evaluate_field_polynomial_at_point(&field_elements, &point),
        WriteableFt63::from_u128(2 * (2u128.pow(2)) + 2 + 0)
    );
}

#[test]
fn test_polynomial_eval_with_elevated_degree() {
    let field_elements = vec![
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
    ];
    let point = WriteableFt63::ONE;
    assert_eq!(
        evaluate_field_polynomial_at_point_with_elevated_degree(&field_elements, &point, 1),
        WriteableFt63::ZERO
    );

    let field_elements = vec![WriteableFt63::ONE, WriteableFt63::ONE, WriteableFt63::ONE];
    assert_eq!(
        evaluate_field_polynomial_at_point_with_elevated_degree(&field_elements, &point, 1),
        WriteableFt63::ONE + WriteableFt63::ONE + WriteableFt63::ONE
    );

    let field_elements = vec![
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
        WriteableFt63::ONE + WriteableFt63::ONE,
    ];
    let point = WriteableFt63::ONE + WriteableFt63::ONE;
    assert_eq!(
        evaluate_field_polynomial_at_point(&field_elements, &point),
        evaluate_field_polynomial_at_point_with_elevated_degree(
            &[WriteableFt63::ONE + WriteableFt63::ONE],
            &point,
            2,
        )
    );

    let field_elements = vec![
        WriteableFt63::ZERO,
        WriteableFt63::ZERO,
        WriteableFt63::ONE + WriteableFt63::ONE,
        WriteableFt63::ONE + WriteableFt63::ONE,
    ];
    let point = WriteableFt63::ONE + WriteableFt63::ONE;
    assert_eq!(
        evaluate_field_polynomial_at_point(&field_elements, &point),
        evaluate_field_polynomial_at_point_with_elevated_degree(
            &[
                WriteableFt63::ONE + WriteableFt63::ONE,
                WriteableFt63::ONE + WriteableFt63::ONE
            ],
            &point,
            2,
        )
    );
}

#[test]
fn bytes_into_then_out_of_field_elements() {
    static LEN: usize = 999;
    let mut bytes = vec![0u8; LEN];
    // fill bytes with random data
    let mut rng = rand::thread_rng();
    for byte in bytes.iter_mut() {
        *byte = rng.gen::<u8>();
    }
    let field = convert_byte_vec_to_field_elements_vec::<WriteableFt63>(&bytes);

    let bytes_back = convert_field_elements_vec_to_byte_vec(&field, LEN);

    assert_eq!(bytes, bytes_back);
}

#[tokio::test]
async fn different_read_to_vecs_are_equivalent() {
    use field_generator_iter::FieldGeneratorIter;
    let files = [
        "test_files/1000_byte_file.bytes".to_string(),
        "test_files/4000_byte_file.bytes".to_string(),
        "test_files/10000_byte_file.bytes".to_string(),
        "test_files/40000_byte_file.bytes".to_string(),
    ];
    // println!("{:?}", std::env::current_dir());
    for file_name in files {
        println!("testing file {}", file_name);
        let original = {
            let mut file = std::fs::File::open(&file_name).unwrap();
            read_file_to_field_elements_vec::<WriteableFt63>(&mut file)
        };
        let rewrite = {
            let mut file = tokio::fs::File::open(&file_name).await.unwrap();
            stream_file_to_field_elements_vec::<WriteableFt63>(&mut file)
                .await
                .unwrap()
        };
        let sync_rewrite = {
            let mut file = std::fs::File::open(&file_name).unwrap();
            stream_file_to_field_elements_vec_sync::<WriteableFt63>(&mut file).unwrap()
        };
        let iter_rewrite: Vec<WriteableFt63> = {
            let file = std::fs::File::open(&file_name).unwrap();
            let reader = std::io::BufReader::new(file).bytes().map(|r| r.unwrap());
            let field_iter = FieldGeneratorIter::new(reader);
            field_iter.collect()
        };
        assert_eq!(original.0, rewrite.0);
        assert_eq!(original.0, sync_rewrite.0);
        for i in 0..original.1.len() {
            assert_eq!(
                original.1[i], rewrite.1[i],
                "failed at index {} on file {}",
                i, file_name
            );
            assert_eq!(
                original.1[i], rewrite.1[i],
                "failed at index {} on file {}",
                i, file_name
            );
            assert_eq!(
                original.1[i], iter_rewrite[i],
                "failed at index {} on file {} for iter",
                i, file_name
            );
        }
        assert_eq!(original.1.len(), rewrite.1.len());
        assert_eq!(original.1.len(), sync_rewrite.1.len());
        assert_eq!(original.1.len(), iter_rewrite.len());
    }
}

#[test]
fn are_fields_the_same() {
    let bytes = RandomBytesIterator::new().take(1000).collect::<Vec<_>>();

    let writeable63_elements = WriteableFt63::from_byte_vec(&bytes);
    let writeable253_elements = Ft253_192::from_byte_vec(&bytes);

    assert_eq!(
        DataField::field_vec_to_byte_vec(&writeable63_elements)[..bytes.len()],
        DataField::field_vec_to_byte_vec(&writeable253_elements)[..bytes.len()]
    );
}
