use std::cmp::min;
use std::fs::File;
use std::io::{Read, Write};
use std::mem;

use ff::PrimeField;
use itertools::Itertools;
use rand::{random, Rng};

pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

pub trait field_bytes<const U64_WIDTH: usize>: PrimeField {
    const BYTE_ORDER: ByteOrder;


    fn from_u64_array(array: [u64; U64_WIDTH]) -> Option<Self>;
    fn to_u64_array(&self) -> [u64; U64_WIDTH];
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
    use crate::fields::{ByteOrder, field_bytes};


    const U64_WIDTH: usize = 1;
    const U8_WIDTH: usize = U64_WIDTH * 8;

    #[derive(PrimeField, Num, Deserialize, Serialize)]
    #[PrimeFieldModulus = "5102708120182849537"]
    #[PrimeFieldGenerator = "10"]
    #[PrimeFieldReprEndianness = "little"]
    pub struct Writeable_Ft63([u64; 1]);

    impl field_bytes<U64_WIDTH> for Writeable_Ft63 {
        const BYTE_ORDER: ByteOrder = ByteOrder::LittleEndian;

        fn from_u64_array(input: [u64; U64_WIDTH]) -> Option<Self> {
            let mut ret = Self(input);
            if ret.is_valid(){
                return Some(ret);
            } else {
                return None;
            }
        }

        fn to_u64_array(&self) -> [u64; U64_WIDTH] {
            self.0
        }
    }
}

pub fn read_file_to_field_elements_vec<F>(path: &str) -> Vec<F>
where
    F: ff::PrimeField,
{
    let mut file = File::open(path).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let field_element_capacity: usize = F::CAPACITY as usize;
    let field_element_byte_width: usize = field_element_capacity / 8;
    let u128_byte_width: usize = mem::size_of::<u128>(); //=16 u8s
    let read_in_byte_width = min(u128_byte_width, field_element_byte_width);

    buffer.chunks(read_in_byte_width)
        .map(|bytes| { //todo need to add from_le variant,
            // i don't know how to determine endianness though atm
            if bytes.len() < u128_byte_width {
                let mut bytes = bytes.to_vec();
                bytes.resize(u128_byte_width, 0);
                let bytes: [u8; mem::size_of::<u128>()] = bytes.try_into().unwrap();
                let number = u128::from_be_bytes(bytes);
                F::from_u128(number)
            } else {
                let number = u128::from_be_bytes(bytes.try_into().unwrap());
                F::from_u128(number)
            }
        })
        .collect()
}

pub fn field_elements_vec_to_file<F>(path: &str, field_elements: &Vec<F>)
where
    F: ff::PrimeField,
{
    let mut file = File::create(path).unwrap();
    let field_element_capacity: usize = F::CAPACITY as usize;
    let field_element_byte_width: usize = field_element_capacity / 8;
    let u128_byte_width: usize = mem::size_of::<u128>(); //=16 u8s
    let write_out_byte_width = min(u128_byte_width, field_element_byte_width);

    for field_element in field_elements {
        let repr = field_element.to_repr();
        let number = repr.as_ref();
        //todo: need to adjust based on BE or LE repr
        let write_buffer = &number[number.len()-write_out_byte_width..];

        //check if we are looking at the last `field_element` in `field_elements`
        //if so, we need to drop trailing zeroes as well
        if field_element == field_elements.last().unwrap() {
            let mut write_buffer = write_buffer.to_vec();
            while write_buffer.last() == Some(&0) {
                write_buffer.pop();
            }
            file.write_all(&write_buffer).unwrap();
        } else {
            file.write_all(write_buffer).unwrap();
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