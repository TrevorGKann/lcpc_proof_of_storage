use std::fs::File;
use std::io::{Read, Write};
use std::mem;

use ff::{Field, PrimeField};
use itertools::Itertools;
use num_traits::{FromBytes, One, ToBytes, Zero};
use num_traits::ops::bytes::NumBytes;
use rand::Rng;
use tracing_subscriber::registry::Data;

use data_field::ByteOrder;
pub use writable_ft63::WriteableFt63;

use crate::fields::data_field::DataField;

pub mod data_field;
mod writable_ft63;
#[derive(Debug)]
pub enum FieldErr {
    InvalidFieldElement,
}

pub mod ft253_192 {
    use ff::PrimeField;
    use ff_derive_num::Num;
    use serde::{Deserialize, Serialize};

    #[derive(PrimeField, Num, Deserialize, Serialize)]
    #[PrimeFieldModulus = "14474011154664524421669271390699307717822958659997404088829842556525106692097"]
    #[PrimeFieldGenerator = "3"]
    #[PrimeFieldReprEndianness = "big"]
    pub struct Ft253_192([u64; 4]);
}

#[tracing::instrument]
pub fn read_file_to_field_elements_vec<F: DataField>(file: &mut File) -> (usize, Vec<F>) {
    // todo: need to convert to async with tokio file
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    tracing::trace!("read {} bytes from file", buffer.len());

    (buffer.len(), convert_byte_vec_to_field_elements_vec(&buffer))
}

#[tracing::instrument]
pub fn convert_byte_vec_to_field_elements_vec<F: DataField>(byte_vec: &[u8]) -> Vec<F> {
    let read_in_bytes = (F::DATA_BYTE_CAPACITY) as usize;


    F::from_byte_vec(byte_vec)
}

#[tracing::instrument]
pub fn convert_field_elements_vec_to_byte_vec<F: DataField>(field_elements: &[F],
                                                            expected_length: usize) -> Vec<u8> {
    let mut bytes = F::field_vec_to_byte_vec(field_elements);
    bytes.truncate(expected_length);
    bytes
}


#[tracing::instrument]
pub fn read_file_path_to_field_elements_vec(path: &str) -> Vec<WriteableFt63>
{
    let mut file = File::open(path).unwrap();
    let (_, result) = read_file_to_field_elements_vec(&mut file);
    result
}

fn byte_array_to_u64_array<const OUTPUT_WIDTH: usize>(input: &[u8], endianness: ByteOrder) -> [u64; OUTPUT_WIDTH] {
    let mut full_length_byte_array = [0u8; mem::size_of::<u64>()];
    match writable_ft63::ENDIANNESS {
        ByteOrder::BigEndian => {
            full_length_byte_array[mem::size_of::<u64>() - input.len()..].copy_from_slice(input);
        }
        ByteOrder::LittleEndian => {
            full_length_byte_array[..input.len()].copy_from_slice(input);
        }
    }
    let mut ret = [0u64; OUTPUT_WIDTH];
    match endianness {
        ByteOrder::BigEndian => {
            ret[0] = u64::from_be_bytes(full_length_byte_array);
        }
        ByteOrder::LittleEndian => {
            ret[0] = u64::from_le_bytes(full_length_byte_array);
        }
    }
    ret
}

fn u64_array_to_byte_array<const INPUT_WIDTH: usize>(input: &[u64; INPUT_WIDTH], endianness: ByteOrder) -> Vec<u8> {
    let mut u8_collection_vector: Vec<u8> = Vec::with_capacity(INPUT_WIDTH * mem::size_of::<u64>());
    for input_byte in input {
        match endianness {
            ByteOrder::BigEndian => {
                u8_collection_vector.extend_from_slice(&input_byte.to_be_bytes());
            }
            ByteOrder::LittleEndian => {
                u8_collection_vector.extend_from_slice(&input_byte.to_le_bytes());
            }
        }
    }
    // let return_value = u8_collection_vector.as_slice();
    // return return_value
    u8_collection_vector
}

pub fn field_elements_vec_to_file(path: &str, field_elements: &Vec<WriteableFt63>)
{
    let mut file = File::create(path).unwrap();

    let write_out_byte_width = (WriteableFt63::CAPACITY / 8) as usize;

    for (index, field_element) in field_elements.iter().enumerate() {
        let mut write_buffer = u64_array_to_byte_array::<1>(&field_element.to_u64_array(), writable_ft63::ENDIANNESS);

        let endianness = writable_ft63::ENDIANNESS;
        while write_buffer.len() > write_out_byte_width {
            match endianness {
                ByteOrder::BigEndian => {
                    write_buffer.remove(0);
                }
                ByteOrder::LittleEndian => {
                    write_buffer.pop();
                }
            }
        }

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

pub fn random_writeable_field_vec<F: DataField>(log_len: usize) -> Vec<F>
{
    use std::iter::repeat_with;

    let mut rng = rand::thread_rng();

    let read_in_bytes = F::DATA_BYTE_CAPACITY * (1 << log_len);
    let byte_vec = repeat_with(|| rng.gen::<u8>()).take(read_in_bytes as usize).collect_vec();
    let random_vector: Vec<F> = F::from_byte_vec(
        &byte_vec
    );

    // //create a vector of u8 arrays with len `read_in_byte_width` and fill it with random bytes
    // let random_vector: Vec<WriteableFt63> = repeat_with(|| {
    //     let random_u8_vector = repeat_with(|| rng.gen::<u8>()).take(read_in_bytes).collect_vec();
    //     let random_u64_array = byte_array_to_u64_array::<1>(&random_u8_vector, WriteableFt63::ENDIANNESS);
    //     WriteableFt63::from_u64_array(random_u64_array).unwrap()
    // }).take(1 << log_len).collect_vec();
    random_vector
}

/// Note that the polynomial is evaluated little endian style, e.g. the array of coefficients should be passed in as
/// [c_0, c_1, ..., c_d] where c_i is the coefficient of x^i.
pub fn evaluate_field_polynomial_at_point(field_elements: &Vec<WriteableFt63>, point: &WriteableFt63) -> WriteableFt63
{
    let mut result = WriteableFt63::zero();
    let mut current_power = WriteableFt63::one();
    for field_element in field_elements {
        result += *field_element * current_power;
        current_power *= point;
    }
    result
}

// evaluates a polynomial collection as if it started at the nth degree. E.g. ([1,2,3],x,3) would be evaluated 
//      as 1x^3 + 2x^4 + 3x^5
pub fn evaluate_field_polynomial_at_point_with_elevated_degree(field_elements: &[WriteableFt63], point: &WriteableFt63, degree_offset: u64) -> WriteableFt63
{
    let mut result = WriteableFt63::zero();
    let mut current_power = point.clone().pow([degree_offset]);
    for field_element in field_elements.iter() {
        result += *field_element * current_power;
        current_power *= point;
    }
    result
}


pub fn vector_multiply<F: PrimeField>(
    a: &[F],
    b: &[F],
) -> F
{
    a.iter().zip(b.iter()).map(|(a, b)| *a * b).sum()
}

pub fn is_power_of_two(x: usize) -> bool {
    x & (x - 1) == 0
}


#[test]
fn test_polynomial_eval() {
    let field_elements = vec![WriteableFt63::zero(), WriteableFt63::zero(), WriteableFt63::zero()];
    let point = WriteableFt63::one();
    assert_eq!(evaluate_field_polynomial_at_point(&field_elements, &point), WriteableFt63::zero());

    let field_elements = vec![WriteableFt63::one(), WriteableFt63::one(), WriteableFt63::one()];
    assert_eq!(evaluate_field_polynomial_at_point(&field_elements, &point), WriteableFt63::one() + WriteableFt63::one() + WriteableFt63::one());

    let field_elements = vec![WriteableFt63::zero(), WriteableFt63::from_u128(1), WriteableFt63::from_u128(2)];
    let point = WriteableFt63::from_u128(2);
    assert_eq!(evaluate_field_polynomial_at_point(&field_elements, &point), WriteableFt63::from_u128(2 * (2u128.pow(2)) + 2 + 0));
}

#[test]
fn test_polynomial_eval_with_elevated_degree() {
    let field_elements = vec![WriteableFt63::zero(), WriteableFt63::zero(), WriteableFt63::zero()];
    let point = WriteableFt63::one();
    assert_eq!(evaluate_field_polynomial_at_point_with_elevated_degree(&field_elements, &point, 1), WriteableFt63::zero());


    let field_elements = vec![WriteableFt63::one(), WriteableFt63::one(), WriteableFt63::one()];
    assert_eq!(evaluate_field_polynomial_at_point_with_elevated_degree(&field_elements, &point, 1), WriteableFt63::one() + WriteableFt63::one() + WriteableFt63::one());


    let field_elements = vec![WriteableFt63::zero(), WriteableFt63::zero(), WriteableFt63::one() + WriteableFt63::one()];
    let point = WriteableFt63::one() + WriteableFt63::one();
    assert_eq!(evaluate_field_polynomial_at_point(
        &field_elements,
        &point,
    ), evaluate_field_polynomial_at_point_with_elevated_degree(
        &[WriteableFt63::one() + WriteableFt63::one()],
        &point,
        2,
    ));

    let field_elements = vec![WriteableFt63::zero(), WriteableFt63::zero(), WriteableFt63::one() + WriteableFt63::one(), WriteableFt63::one() + WriteableFt63::one()];
    let point = WriteableFt63::one() + WriteableFt63::one();
    assert_eq!(evaluate_field_polynomial_at_point(
        &field_elements,
        &point,
    ), evaluate_field_polynomial_at_point_with_elevated_degree(
        &[WriteableFt63::one() + WriteableFt63::one(), WriteableFt63::one() + WriteableFt63::one()],
        &point,
        2,
    ));
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