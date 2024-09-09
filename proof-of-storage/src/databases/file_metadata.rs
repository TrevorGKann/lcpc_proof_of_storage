use std::fmt;

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
}

impl FileMetadata {
    pub fn get_file_columns(&self) -> usize {
        self.num_encoded_columns / 2
    }

    pub fn get_end_coordinates(&self) -> (usize, usize) {
        (self.num_rows, self.filesize_in_bytes % self.num_encoded_columns)
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