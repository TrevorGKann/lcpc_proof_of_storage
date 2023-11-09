use std::cmp::min;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::mem;

use ff::PrimeField;
use itertools::Itertools;
use rand::{random, Rng};
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
enum FieldErr {
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
    use ff::{Field, PrimeField, FromUniformBytes};
    use ff_derive_num::Num;
    use serde::{Deserialize, Serialize};
    use lcpc_test_fields::ft63::Ft63;
    use crate::fields::{ByteOrder, FieldBytes, FieldErr};


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
            let mut ret = Self(input);
            if ret.is_valid() {
                return Ok(ret);
            } else {
                return Err(FieldErr::InvalidFieldElement);
            }
        }

        pub fn to_u64_array(&self) -> [u64; 1] {
            self.0
        }
    }
}

pub fn read_file_to_field_elements_vec(path: &str) -> Vec<WriteableFt63>
{
    let mut file = File::open(path).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let read_in_bytes = (WriteableFt63::CAPACITY / 8) as usize;

    buffer.chunks(read_in_bytes)
        .map(|bytes| { //todo need to add from_le/from_be variants
            let mut full_length_byte_array = [0u8; mem::size_of::<u64>()];
            match writable_ft63::ENDIANNESS {
                ByteOrder::BigEndian => {
                    full_length_byte_array[mem::size_of::<u64>() - bytes.len()..].copy_from_slice(bytes);
                },
                ByteOrder::LittleEndian => {
                    full_length_byte_array[..bytes.len()].copy_from_slice(bytes);
                }
            }
            let u64_array = [u64::from_le_bytes(full_length_byte_array)];
            WriteableFt63::from_u64_array(u64_array).unwrap()
            // if bytes.len() < read_in_bytes {
            //     let mut bytes = bytes.to_vec();
            //     bytes.resize(u64_byte_width, 0);
            //     let bytes: [u8; mem::size_of::<u128>()] = bytes.try_into().unwrap();
            //     let number = u128::from_be_bytes(bytes);
            //     F::from_u128(number)
            // } else {
            //     let number = u128::from_be_bytes(bytes.try_into().unwrap());
            //     F::from_u128(number)
            // }
        })
        .collect()
}

fn byte_array_to_u64_array<const output_width: usize>(input: &[u8], endianness: ByteOrder) -> [u64;output_width]{
    let mut ret = [0u64;output_width];
    match endianness {
        ByteOrder::BigEndian => {
            ret[0] = u64::from_be_bytes(input.try_into().unwrap());
        },
        ByteOrder::LittleEndian => {
            ret[0] = u64::from_le_bytes(input.try_into().unwrap());
        }
    }
    ret
}

fn u64_array_to_byte_array<'a, const input_width: usize>(input: &[u64; input_width], endianness: ByteOrder) -> Vec<u8> {
    let mut u8_collection_vector: Vec<u8> = Vec::with_capacity(input_width * mem::size_of::<u64>());
    for i in 0..input.len() {
        match endianness {
            ByteOrder::BigEndian => {
                u8_collection_vector.extend_from_slice(&input[i].to_be_bytes());
            },
            ByteOrder::LittleEndian => {
                u8_collection_vector.extend_from_slice(&input[i].to_le_bytes());
            }
        }
    }
    // let return_value = u8_collection_vector.as_slice();
    // return return_value
    return u8_collection_vector
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
                },
                ByteOrder::LittleEndian => {
                    write_buffer.pop();
                }
            }
        }

        //check if we are looking at the last `field_element` in `field_elements`
        //if so, we need to drop trailing zeroes as well
        if index == field_elements.len()-1 {
            while write_buffer.last() == Some(&0) {
                write_buffer.pop();
            }
            file.write_all(&write_buffer).unwrap();
        } else {
            file.write_all(&write_buffer).unwrap();
        }
    }

}

pub fn random_writeable_field_vec<F>(log_len: usize) -> Vec<F>
where
    F: ff::PrimeField,
{
    use std::iter::repeat_with;

    let mut rng = rand::thread_rng();

    let field_element_capacity: usize = F::CAPACITY as usize;
    let field_element_byte_width: usize = field_element_capacity / 8;
    let u128_byte_width: usize = mem::size_of::<u128>(); //=16 u8s
    let read_in_byte_width = min(u128_byte_width, field_element_byte_width);
    let max_value = 1u128 << (read_in_byte_width*8);

    //create a vector of u8 arrays with len `read_in_byte_width` and fill it with random bytes
    let random_vector: Vec<F> = repeat_with(|| {
        let random_value = rng.gen_range(0..max_value);
        F::from_u128(random_value)
    }).take(1 << log_len).collect_vec();
    return random_vector
}