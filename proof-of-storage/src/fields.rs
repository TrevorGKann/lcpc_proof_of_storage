use std::fs::File;
use std::io::{Read, Write};
use std::mem;

use ff::PrimeField;
use itertools::Itertools;
use num_traits::{One, Zero};
use rand::Rng;

use crate::fields::writable_ft63::WriteableFt63;

pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

pub trait FieldBytes: PrimeField {
    const BYTE_ORDER: ByteOrder;
    const U64_WIDTH: usize;

    fn from_u8_array(array: &[u8]) -> Option<Self>;

    fn to_u8_array(&self) -> &[u64];

    fn from_u64_array(array: &[u64]) -> Option<Self>;
    fn to_u64_array(&self) -> &[u64];
}

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

pub mod writable_ft63 {
    use ff::PrimeField;
    use ff_derive_num::Num;
    use serde::{Deserialize, Serialize};

    use crate::fields::{ByteOrder, FieldErr};

    pub const U64_WIDTH: usize = 1;
    pub const U8_WIDTH: usize = U64_WIDTH * 8;
    pub const ENDIANNESS: ByteOrder = ByteOrder::LittleEndian;

    #[derive(PrimeField, Num, Deserialize, Serialize)]
    #[PrimeFieldModulus = "5102708120182849537"]
    #[PrimeFieldGenerator = "10"]
    #[PrimeFieldReprEndianness = "little"]
    pub struct WriteableFt63([u64; 1]);

    impl WriteableFt63 {
        pub fn from_u64_array(input: [u64; U64_WIDTH]) -> Result<Self, FieldErr> {
            let ret = Self(input);
            if ret.is_valid() {
                Ok(ret)
            } else {
                Err(FieldErr::InvalidFieldElement)
            }
        }

        pub fn to_u64_array(&self) -> [u64; 1] {
            self.0
        }
    }
}

#[tracing::instrument]
pub fn read_file_to_field_elements_vec(file: &mut File) -> (usize, Vec<WriteableFt63>) {
    // todo: need to convert to async with tokio file
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    tracing::trace!("read {} bytes from file", buffer.len());

    (buffer.len(), convert_byte_vec_to_field_elements_vec(&buffer))
}

#[tracing::instrument]
pub fn convert_byte_vec_to_field_elements_vec(byte_vec: &Vec<u8>) -> Vec<WriteableFt63> {
    let read_in_bytes = (WriteableFt63::CAPACITY / 8) as usize;


    byte_vec.chunks(read_in_bytes)
        .map(|bytes| { //todo need to add from_le/from_be variants
            let mut full_length_byte_array = [0u8; mem::size_of::<u64>()];
            match writable_ft63::ENDIANNESS {
                ByteOrder::BigEndian => {
                    full_length_byte_array[mem::size_of::<u64>() - bytes.len()..].copy_from_slice(bytes);
                }
                ByteOrder::LittleEndian => {
                    full_length_byte_array[..bytes.len()].copy_from_slice(bytes);
                }
            }
            let u64_array = [u64::from_le_bytes(full_length_byte_array)];
            WriteableFt63::from_u64_array(u64_array).unwrap()
        })
        .collect()
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

pub fn random_writeable_field_vec(log_len: usize) -> Vec<WriteableFt63>
{
    use std::iter::repeat_with;

    let mut rng = rand::thread_rng();

    let read_in_bytes = (WriteableFt63::CAPACITY / 8) as usize;

    //create a vector of u8 arrays with len `read_in_byte_width` and fill it with random bytes
    let random_vector: Vec<WriteableFt63> = repeat_with(|| {
        let random_u8_vector = repeat_with(|| rng.gen::<u8>()).take(read_in_bytes).collect_vec();
        let random_u64_array = byte_array_to_u64_array::<1>(&random_u8_vector, writable_ft63::ENDIANNESS);
        WriteableFt63::from_u64_array(random_u64_array).unwrap()
    }).take(1 << log_len).collect_vec();
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


pub fn vector_multiply(
    a: &[WriteableFt63],
    b: &[WriteableFt63],
) -> WriteableFt63
{
    a.iter().zip(b.iter()).map(|(a, b)| *a * b).sum()
}

pub fn is_power_of_two(x: usize) -> bool {
    x & (x - 1) == 0
}
