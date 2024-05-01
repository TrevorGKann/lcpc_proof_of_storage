use std::fmt;

use blake3::Hasher as Blake3;
use blake3::traits::digest::Output;
use fffft::FFTError;
use serde::{Deserialize, Serialize};
use tokio::fs;

use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::{fields, PoSCommit, PoSEncoding, PoSField, PoSRoot};
use crate::fields::writable_ft63::WriteableFt63;
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
    pub num_rows: usize,
    pub num_columns: usize,
    pub num_encoded_columns: usize,
    pub filesize_in_bytes: usize,
    pub stored_server: ServerHost,
    pub root: PoSRoot,
}

impl ClientOwnedFileMetadata {
    pub fn get_file_columns(&self) -> usize {
        self.num_encoded_columns / 2
    }

    pub fn get_end_coordinates(&self) -> (usize, usize) {
        (self.num_rows, self.filesize_in_bytes % self.num_encoded_columns)
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerOwnedFileMetadata {
    pub filename: String,
    pub owner: String,
    pub commitment: PoSCommit,
}


#[tracing::instrument]
pub async fn read_server_file_database_from_disk(file_path: String) -> Vec<ServerOwnedFileMetadata> {
    let file_data_result = fs::read(file_path).await;
    if file_data_result.is_err() {
        return vec![]; //todo should probably handle error
    }
    let file_data = file_data_result.unwrap();

    let json = serde_json::from_slice(&file_data).unwrap_or(serde_json::json!({}));

    let file_data_array: Vec<ServerOwnedFileMetadata>;

    if let Some(files) = json.get("files_on_server") {
        file_data_array = serde_json::from_str(files.as_str().unwrap()).unwrap();
    } else {
        file_data_array = vec![];
    }

    file_data_array
}

#[tracing::instrument]
pub async fn write_server_file_database_to_disk(file_path: String, file_data_array: Vec<ServerOwnedFileMetadata>) {
    let file_json = serde_json::to_string(&file_data_array).unwrap();
    let combined_json = serde_json::json!({
        "files_on_server": file_json
    });
    fs::write(file_path, combined_json.to_string()).await.unwrap();
}


// TESTS //
#[tokio::test]
async fn test_client_owned_file_metadata() {
    let fake_filedata = fields::random_writeable_field_vec(20);
    let data_min_width = (fake_filedata.len() as f32).sqrt().ceil() as usize;
    let data_realized_width = data_min_width.next_power_of_two();
    let matrix_colums = (data_realized_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&fake_filedata, &encoding).unwrap();
    let root = commit.get_root();

    let server = ServerHost {
        server_name: Some("test_server".to_string()),
        server_ip: "0.0.0.0".to_string(),
        server_port: 8080,
    };
    let file = ClientOwnedFileMetadata {
        filename: "test_file".to_string(),
        num_rows: 100,
        num_columns: 128,
        num_encoded_columns: 256,
        filesize_in_bytes: 12800,
        stored_server: server.clone(),
        root,
    };
    write_client_file_database_to_disk("test_file_db.json".to_string(), vec![server.clone()], vec![file.clone()]).await;
    let (servers, files) = read_client_file_database_from_disk("test_file_db.json".to_string()).await;
    assert_eq!(servers.len(), 1);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, file.filename);
    assert_eq!(files[0].num_rows, file.num_rows);
    assert_eq!(files[0].num_encoded_columns, file.num_encoded_columns);
    assert_eq!(files[0].num_columns, file.num_columns);
    assert_eq!(files[0].filesize_in_bytes, file.filesize_in_bytes);
    assert_eq!(files[0].root.root, file.root.root);

    assert_eq!(files[0].stored_server.server_name.as_ref().unwrap(), server.server_name.as_ref().unwrap());
    assert_eq!(files[0].stored_server.server_ip, server.server_ip);
    assert_eq!(files[0].stored_server.server_port, server.server_port);
    fs::remove_file("test_file_db.json").await.unwrap();
}

//todo need methods to search the database for existing files and retrieve their commits