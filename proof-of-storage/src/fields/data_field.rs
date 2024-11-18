use ff::PrimeField;
use num_traits::ops::bytes::NumBytes;

pub trait DataField: PrimeField {
    /// Specifies the total bytes that can be stored in the field elements without loss.
    ///     Because of the modulus bits, and to efficiently use bytes (at some loss of total bit
    ///     capacity), the DataBytes will be smaller than the struct used to store the Field.
    type DataBytes: NumBytes;

    /// The number of bytes that represent data. A shortcut for the size of DataBytes in bytes.
    ///     The following quantities **MUST** be equal
    ///         - `<Self as PrimeField>::CAPACITY / 8`
    ///         - std::mem::size_of::<DataBytes>()
    const DATA_BYTE_CAPACITY: u32 = <Self as PrimeField>::CAPACITY / 8;

    fn test_type_sizes_are_correct() -> bool {
        (<Self as PrimeField>::CAPACITY / 8) as usize == std::mem::size_of::<Self::DataBytes>()
            && Self::DATA_BYTE_CAPACITY == (<Self as PrimeField>::CAPACITY / 8)
            && (Self::DATA_BYTE_CAPACITY as usize) < std::mem::size_of::<Self>()
    }
    const ENDIANNESS: ByteOrder;
    fn from_data_bytes(buf: &Self::DataBytes) -> Self;
    fn to_data_bytes(&self) -> Self::DataBytes;

    fn from_byte_vec(vec: &[Self::DataBytes]) -> Vec<Self> {
        vec.iter().map(|bytes| Self::from_data_bytes(bytes)).collect::<Vec<Self>>()
    }
    fn field_vec_to_byte_vec(field_vec: &[Self]) -> Vec<u8> {
        field_vec.iter()
            .flat_map(|field_element| field_element.to_data_bytes().as_ref().to_owned())
            .collect::<Vec<u8>>()
    }
}


pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}