use std::fs::File;
use std::io::Read;
use ff::PrimeField;
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

    buffer.chunks(field_element_byte_width)
        .map(|bytes| {
            let mut number = u128::from_be_bytes(bytes.try_into().unwrap());
            F::from_u128(number)
        })
        .collect()
}