use std::fmt;

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::networking::shared::ServerMessages;

#[derive(Debug, Serialize, Deserialize, Clone)]
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

impl fmt::Display for FileMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stored_server.server_name.as_ref().is_some() {
            write!(f, "File: {} - {} total bytes, stored on \"{}\"", self.filename, self.filesize_in_bytes, self.stored_server.server_name.as_ref().unwrap())
        } else {
            write!(f, "File: \"{}\" - {} total bytes, stored at {}:{}", self.filename, self.filesize_in_bytes, self.stored_server.server_ip, self.stored_server.server_port)
        }
    }
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
pub async fn read_file_database_from_disk(file_path: String) -> (Vec<ServerMetaData>, Vec<FileMetadata>) {
    let file_data_result = fs::read(file_path).await;
    if file_data_result.is_err() {
        return (vec![], vec![]);
    }
    let file_data = file_data_result.unwrap();

    let json = serde_json::from_slice(&file_data).unwrap_or(serde_json::json!({}));

    let server_data_array: Vec<ServerMetaData>;
    let file_data_array: Vec<FileMetadata>;

    if let Some(servers) = json.get("servers") {
        server_data_array = serde_json::from_str(servers.as_str().unwrap()).unwrap();
    } else {
        server_data_array = vec![];
    }

    if let Some(files) = json.get("files") {
        file_data_array = serde_json::from_str(files.as_str().unwrap()).unwrap();
    } else {
        file_data_array = vec![];
    }

    (server_data_array, file_data_array)
}

#[tracing::instrument]
pub async fn write_file_database_to_disk(file_path: String, server_data_array: Vec<ServerMetaData>, file_data_array: Vec<FileMetadata>) {
    let server_json = serde_json::to_string(&server_data_array).unwrap();
    let file_json = serde_json::to_string(&file_data_array).unwrap();
    let combined_json = serde_json::json!({
        "servers": server_json,
        "files": file_json
    });
    fs::write(file_path, combined_json.to_string()).await.unwrap();
}

#[tokio::test]
async fn test_file_metadata() {
    let server = ServerMetaData {
        server_name: Some("test_server".to_string()),
        server_ip: "0.0.0.0".to_string(),
        server_port: 8080,
    };
    let file = FileMetadata {
        filename: "test_file".to_string(),
        rows: 100,
        encoded_columns: 1000,
        filesize_in_bytes: 100000,
        stored_server: server.clone(),
    };
    write_file_database_to_disk("test_file_db.json".to_string(), vec![server], vec![file]).await;
    let (servers, files) = read_file_database_from_disk("test_file_db.json".to_string()).await;
    assert_eq!(servers.len(), 1);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, "test_file");
    assert_eq!(files[0].rows, 100);
    assert_eq!(files[0].encoded_columns, 1000);
    assert_eq!(files[0].filesize_in_bytes, 100000);

    assert_eq!(files[0].stored_server.server_name.as_ref().unwrap(), "test_server");
    assert_eq!(files[0].stored_server.server_ip, "0.0.0.0");
    assert_eq!(files[0].stored_server.server_port, 8080);
    fs::remove_file("test_file_db.json").await.unwrap();
}
