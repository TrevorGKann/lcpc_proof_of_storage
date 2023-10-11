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

fn read_file_to_field_elements_vec<F>(path: &str) -> Vec<F>
where
    F: ff::PrimeField,
{
    let mut file = File::open(path).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let field_element_capacity = ft253_192::Ft253_192::CAPACITY;
    let field_element_width = ft253_192::Ft253_192::NUM_BITS / 64;

    let mut data = Vec::new();
    for i in 0..(buffer.len() / 32) {
        let mut tmp = [0u8; 32];
        tmp.copy_from_slice(&buffer[i * 32..(i + 1) * 32]);
        let tmp = F::from_random_bytes(&tmp).unwrap();
        data.push(tmp);
    }
    data
}