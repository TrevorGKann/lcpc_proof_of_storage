use ff::PrimeField;
use num_traits::ops::bytes::NumBytes;

/// Interface to allow for data storage, retrieval, and usage from PrimeFields without loss.
/// Loses some of the storage capacity of the native field (due to modulus bits and use of bytes
/// for native CPU efficiency).
///
/// Users have to assign how bytes get stored within and retrieved from the field without loss. i.e,
/// if your field natively represents as `[u64; 1]` you have to tell which bytes of that u64 to
/// fill and take from.   
pub trait DataField: PrimeField {
    /// Specifies the total bytes that can be stored in the field elements without loss.
    ///     Because of the modulus bits, and to efficiently use bytes (at some loss of total bit
    ///     capacity), the DataBytes will be smaller than the struct used to store the Field.
    type DataBytes: NumBytes + Default;

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

    /// Converts arbitrarily long byte vectors into the corresponding number of field elements.
    /// Will zero pad the last element if the number of bytes doesn't evenly divide
    /// `DATA_BYTE_CAPACITY`
    fn from_byte_vec(vec: &[u8]) -> Vec<Self> {
        vec.chunks(Self::DATA_BYTE_CAPACITY as usize)
            .map(|byte_chunk| {
                let mut byte_array: Self::DataBytes = Self::DataBytes::default();
                byte_array.as_mut()[..byte_chunk.len()].clone_from_slice(byte_chunk);
                Self::from_data_bytes(&byte_array)
            })
            .collect::<Vec<Self>>()
    }

    /// Converts a vec of field elements to at least the corresponding data that fills it in
    /// bytes. Will zero pad the ending byte array if the original bytes were insufficiently long.
    ///
    /// For example, if `DataBytes` is a `[u8; 2]` and the byte string `[1,2,3]` is fed into
    /// `from_byte_vec` and then back from `field_vec_to_byte_vec`, the user would get `[1,2,3,0]`.
    /// In other words, if ending zero padding matters, the user must keep track of the
    /// total byte count.
    fn field_vec_to_byte_vec(field_vec: &[Self]) -> Vec<u8> {
        field_vec
            .iter()
            .flat_map(|field_element| field_element.to_data_bytes().as_ref().to_owned())
            .collect::<Vec<u8>>()
    }
}

pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}
