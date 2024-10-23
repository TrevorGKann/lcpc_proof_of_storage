use std::fmt;

use ff::PrimeField;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::databases::server_host::ServerHost;
use crate::PoSRoot;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMetadata {
    pub id_ulid: Ulid,
    pub filename: String,
    pub num_rows: usize,
    pub num_columns: usize,
    pub num_encoded_columns: usize,
    pub filesize_in_bytes: usize,
    pub stored_server: ServerHost,
    pub root: PoSRoot,
    // todo: refactor add: start_byte_offset
}

impl FileMetadata {
    pub fn get_end_coefficient_coordinates<F: PrimeField>(&self) -> (usize, usize) {
        (self.num_rows, self.filesize_in_bytes % (self.num_encoded_columns * (F::CAPACITY as usize / 8)))
    }

    pub fn get_last_coefficient_write_byte_offset<F: PrimeField>(&self) -> usize {
        self.filesize_in_bytes % (F::CAPACITY as usize / 8)
    }

    pub fn bytes_in_a_row<F: PrimeField>(&self) -> usize {
        self.num_columns * (F::CAPACITY as usize / 8)
    }
}

impl fmt::Display for FileMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stored_server.server_name.as_ref().is_some() {
            write!(f, "File: {} - {} total bytes, stored on \"{}\"", self.filename, self.filesize_in_bytes, self.stored_server.server_name.as_ref().unwrap())
        } else {
            write!(f, "File: \"{}\" - {} total bytes, stored at {}:{}", self.filename, self.filesize_in_bytes, self.stored_server.server_ip, self.stored_server.server_port)
        }
    }
}