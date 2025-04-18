use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use ulid::Ulid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncodedFileMetadata {
    pub ulid: Ulid,
    pub pre_encoded_size: usize,
    pub encoded_size: usize,
    pub rows_written: usize,
    pub row_capacity: usize,
    pub bytes_of_data: usize,
}

impl EncodedFileMetadata {
    pub fn write_to_file(&self, writable: &mut impl Write) -> std::io::Result<()> {
        let self_as_bytes = serde_json::to_string(self)?;
        writable.write_all(self_as_bytes.as_bytes())
    }

    pub fn read_from_file(readable: &mut impl Read) -> std::io::Result<Self> {
        let mut read_values = String::new();
        readable.read_to_string(&mut read_values)?;
        let new_self = serde_json::from_str(read_values.as_str())?;
        Ok(new_self)
    }
}
