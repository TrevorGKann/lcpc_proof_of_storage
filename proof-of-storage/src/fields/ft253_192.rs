use crate::fields::data_field::{ByteOrder, DataField};
use ff::PrimeField;
use ff_derive_num::Num;
use serde::{Deserialize, Serialize};

#[derive(PrimeField, Num, Deserialize, Serialize)]
#[PrimeFieldModulus = "14474011154664524421669271390699307717822958659997404088829842556525106692097"]
#[PrimeFieldGenerator = "3"]
#[PrimeFieldReprEndianness = "big"]
pub struct Ft253_192([u64; 4]);

impl DataField for Ft253_192 {
    /// prime modulus for this struct takes up 32 bytes with the last byte incomplete, so we can
    /// store 31 bytes of data in it.
    type DataBytes = [u8; 31];
    const ENDIANNESS: ByteOrder = ByteOrder::BigEndian;

    fn from_data_bytes(buf: &Self::DataBytes) -> Self {
        let mut zero_padded = [0u8; 32];
        zero_padded[..31].copy_from_slice(buf);

        let mut internal_u64_arrangement = [0u64; 4];
        internal_u64_arrangement
            .iter_mut()
            .enumerate()
            .for_each(|(i, x)| {
                *x = u64::from_be_bytes(zero_padded[i * 8..(i + 1) * 8].try_into().unwrap());
            });
        Ft253_192(internal_u64_arrangement)
    }

    fn to_data_bytes(&self) -> Self::DataBytes {
        let mut output = [0u8; 31];
        for (i, chunk) in self.0.iter().enumerate() {
            if i < 3 {
                output[i * 8..(i + 1) * 8].copy_from_slice(&chunk.to_be_bytes());
            } else {
                output[i * 8..(i + 1) * 8 - 1].copy_from_slice(&chunk.to_be_bytes()[..7]);
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use crate::fields::data_field::DataField;
    use crate::fields::ft253_192::Ft253_192;
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    #[test]
    fn test_type_sizes_are_correct() {
        assert!(Ft253_192::test_type_sizes_are_correct())
    }

    #[test]
    fn into_and_out_of_bytes() {
        let mut seed = ChaCha8Rng::seed_from_u64(1337);
        for _ in 0..1000 {
            let mut random_data = [0u8; 31];
            seed.fill_bytes(&mut random_data);

            let field_element = Ft253_192::from_data_bytes(&random_data);
            let bytes_back = field_element.to_data_bytes();
            assert_eq!(bytes_back, random_data);
            assert_eq!(field_element, Ft253_192::from_data_bytes(&bytes_back));
        }
    }
}
