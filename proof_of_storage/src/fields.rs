use std::cmp::min;
use std::fs::File;
use std::io::Read;
use std::mem;
use ff::PrimeField;
use itertools::Itertools;
use crate::fields::ft253_192::Ft253_192;

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

pub(crate) fn read_file_to_field_elements_vec<F>(path: &str) -> Vec<F>
where
    F: ff::PrimeField,
{
    let mut file = File::open(path).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let field_element_capacity: usize = ft253_192::Ft253_192::CAPACITY as usize;
    let field_element_byte_width: usize = field_element_capacity / 8;
    let u128_byte_width: usize = mem::size_of::<u128>(); //=16 u8s
    let read_in_byte_width = min(u128_byte_width, field_element_byte_width);
    // let debug_number = u128::from_be_bytes([0;15]);

    buffer.chunks(read_in_byte_width)
        .map(|bytes| { //todo need to add from le variant
            println!("bytes: {:?}", bytes);
            if bytes.len() < u128_byte_width {
                let mut bytes = bytes.to_vec();
                bytes.resize(u128_byte_width, 0);
                let bytes: [u8; mem::size_of::<u128>()] = bytes.try_into().unwrap();
                let mut number = u128::from_be_bytes(bytes);
                F::from_u128(number)
            } else {
                let mut number = u128::from_be_bytes(bytes.try_into().unwrap());
                F::from_u128(number)
            }
        })
        .collect()
}