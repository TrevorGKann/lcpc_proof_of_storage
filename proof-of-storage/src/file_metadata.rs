use std::fmt;

use fffft::FFTError;
use serde::{Deserialize, Serialize};
use tokio::fs;

use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::LigeroEncoding;

use crate::{PoSCommit, PoSEncoding, PoSField};
use crate::networking::shared::ServerMessages;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerHost {
    pub server_name: Option<String>,
    pub server_ip: String,
    pub server_port: u16,
}

impl fmt::Display for ServerHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.server_name {
            write!(f, "Server: \"{}\" at {}:{}", name.to_string(), self.server_ip, self.server_port)
        } else {
            write!(f, "Server: {}:{}", self.server_ip, self.server_port)
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientOwnedFileMetadata {
    pub filename: String,
    pub rows: usize,
    pub columns: usize,
    pub encoded_columns: usize,
    //todo ought to be deprecated
    pub filesize_in_bytes: usize,
    pub stored_server: ServerHost,
}

impl ClientOwnedFileMetadata {
    pub fn get_file_columns(&self) -> usize {
        self.encoded_columns / 2
    }

    pub fn get_end_coordinates(&self) -> (usize, usize) {
        (self.rows, self.filesize_in_bytes % self.encoded_columns)
    }
}

impl fmt::Display for ClientOwnedFileMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stored_server.server_name.as_ref().is_some() {
            write!(f, "File: {} - {} total bytes, stored on \"{}\"", self.filename, self.filesize_in_bytes, self.stored_server.server_name.as_ref().unwrap())
        } else {
            write!(f, "File: \"{}\" - {} total bytes, stored at {}:{}", self.filename, self.filesize_in_bytes, self.stored_server.server_ip, self.stored_server.server_port)
        }
    }
}

#[tracing::instrument]
pub async fn read_client_file_database_from_disk(file_path: String) -> (Vec<ServerHost>, Vec<ClientOwnedFileMetadata>) {
    let file_data_result = fs::read(file_path).await;
    if file_data_result.is_err() {
        return (vec![], vec![]);
    }
    let file_data = file_data_result.unwrap();

    let json = serde_json::from_slice(&file_data).unwrap_or(serde_json::json!({}));

    let server_data_array: Vec<ServerHost>;
    let file_data_array: Vec<ClientOwnedFileMetadata>;

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
pub async fn write_client_file_database_to_disk(file_path: String, server_data_array: Vec<ServerHost>, file_data_array: Vec<ClientOwnedFileMetadata>) {
    let server_json = serde_json::to_string(&server_data_array).unwrap();
    let file_json = serde_json::to_string(&file_data_array).unwrap();
    let combined_json = serde_json::json!({
        "servers": server_json,
        "files": file_json
    });
    fs::write(file_path, combined_json.to_string()).await.unwrap();
}

pub struct ServerOwnedFileMetadata {
    pub filename: String,
    pub owner: String,
    pub commitment: PoSCommit,
}

// TESTS //
#[tokio::test]
async fn test_client_owned_file_metadata() {
    let server = ServerHost {
        server_name: Some("test_server".to_string()),
        server_ip: "0.0.0.0".to_string(),
        server_port: 8080,
    };
    let file = ClientOwnedFileMetadata {
        filename: "test_file".to_string(),
        rows: 100,
        columns: 128,
        encoded_columns: 256,
        filesize_in_bytes: 12800,
        stored_server: server.clone(),
    };
    write_client_file_database_to_disk("test_file_db.json".to_string(), vec![server.clone()], vec![file.clone()]).await;
    let (servers, files) = read_client_file_database_from_disk("test_file_db.json".to_string()).await;
    assert_eq!(servers.len(), 1);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, file.filename);
    assert_eq!(files[0].rows, file.rows);
    assert_eq!(files[0].encoded_columns, file.encoded_columns);
    assert_eq!(files[0].columns, file.columns);
    assert_eq!(files[0].filesize_in_bytes, file.filesize_in_bytes);

    assert_eq!(files[0].stored_server.server_name.as_ref().unwrap(), server.server_name.as_ref().unwrap());
    assert_eq!(files[0].stored_server.server_ip, server.server_ip);
    assert_eq!(files[0].stored_server.server_port, server.server_port);
    fs::remove_file("test_file_db.json").await.unwrap();
}
