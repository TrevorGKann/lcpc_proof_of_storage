use ff::PrimeField;
use ff_derive_num::Num;
use serde::{Deserialize, Serialize};

use crate::fields::data_field::{ByteOrder, DataField};
use crate::fields::FieldErr;

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

    pub const BYTE_CAPACITY: u32 = Self::CAPACITY / 8;
}


impl DataField for WriteableFt63 {
    type DataBytes = [u8; 7];
    const ENDIANNESS: ByteOrder = ByteOrder::LittleEndian;

    fn from_data_bytes(buf: &Self::DataBytes) -> Self {
        let mut zero_padded = [0u8; 8];
        zero_padded[..7].copy_from_slice(buf);
        let internal_u64_arrangement = [u64::from_le_bytes(zero_padded)];
        WriteableFt63(internal_u64_arrangement)
    }

    fn to_data_bytes(&self) -> Self::DataBytes {
        let mut return_array: [u8; 7] = [0u8; 7];
        return_array.copy_from_slice(&self.0[0].to_le_bytes()[0..8]);
        return_array
    }
}
