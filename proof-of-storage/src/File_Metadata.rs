use std::fmt;
use serde::{Deserialize, Serialize};
use tokio::fs;
use crate::networking::shared::ServerMessages;

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerMetaData {
    pub server_name: Option<String>,
    pub server_ip: String,
    pub server_port: u16,
}

impl fmt::Display for ServerMetaData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.server_name {
            write!(f, "Server: \"{}\" at {}:{}", name.to_string(), self.server_ip, self.server_port)
        } else {
            write!(f, "Server: {}:{}", self.server_ip, self.server_port)
        }
    }
}


#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub filename: String,
    pub rows: usize,
    pub encoded_columns: usize,
    pub filesize_in_bytes: usize,
    pub stored_server: ServerMetaData,
}

impl FileMetadata {
    pub fn get_file_columns(&self) -> usize {
        self.encoded_columns / 2
    }

    pub fn get_end_coordinates(&self) -> (usize, usize) {
        (self.rows, self.filesize_in_bytes % self.encoded_columns)
    }
}

#[tracing::instrument]
pub async fn read_file_database_from_disk(file_path: String) -> (Vec<ServerMetaData>, Vec<FileMetadata>){
    let file_data = fs::read(file_path).await.unwrap();
    // let json = serde_json::from_slice(&file_data).unwrap();
    unimplemented!()
}

#[tracing::instrument]
pub async fn write_file_database_to_disk(file_path: String, server_data_array: Vec<ServerMetaData>, file_data_array: Vec<FileMetadata>){
    let server_json = serde_json::to_string(&server_data_array).unwrap();
    let file_json = serde_json::to_string(&file_data_array).unwrap();
    let combined_json = serde_json::json!({
        "servers": server_json,
        "files": file_json
    });
    tokio::fs::write(file_path, combined_json.to_string()).await.unwrap();
}