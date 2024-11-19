use std::fs::File;
use std::io::{Read, Write};

use ff::{Field, PrimeField};
pub use ft253_192::Ft253_192;
use itertools::Itertools;
use num_traits::{One, ToBytes, Zero};
use rand::Rng;
use tracing_subscriber::registry::Data;
pub use writable_ft63::WriteableFt63;

use crate::fields::data_field::DataField;

pub mod data_field;
mod ft253_192;
mod writable_ft63;

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
pub fn convert_byte_vec_to_field_elements_vec<F: DataField>(byte_vec: &[u8]) -> Vec<F> {
    let read_in_bytes = (F::DATA_BYTE_CAPACITY) as usize;

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

    let write_out_byte_width = (F::DATA_BYTE_CAPACITY) as usize;

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
        evaluate_field_polynomial_at_point(&field_elements, &point,),
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
        evaluate_field_polynomial_at_point(&field_elements, &point,),
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
