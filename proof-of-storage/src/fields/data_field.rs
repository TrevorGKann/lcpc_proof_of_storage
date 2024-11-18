use ff::PrimeField;
use num_traits::ops::bytes::NumBytes;

pub trait DataField: PrimeField {
    /// Specifies the total bytes needed per field element.
    ///     Note, importantly, that the bits will not all be used. DataFields do not efficiently
    ///     store data. Use `DATA_BYTES` to get the bytes that represent data.
    type Bytes: NumBytes;

    /// The number of bytes that represent data. At least one byte used for the modulus.
    const DATA_BYTES: u32;
    const ENDIANNESS: ByteOrder;
    fn from_bytes(buf: &Self::Bytes) -> Self;
    fn to_bytes(&self) -> Self::Bytes;

    fn from_byte_vec(vec: &[Self::Bytes]) -> Vec<Self> {
        vec.iter().map(|bytes| Self::from_bytes(bytes)).collect::<Vec<Self>>()
    }
    fn self_vec_to_byte_vec(self_vec: &[Self]) -> Vec<u8> {
        self_vec.iter()
            .flat_map(|self_elem| self_elem.to_bytes().as_ref().to_owned())
            .collect::<Vec<u8>>()
    }
}


pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}