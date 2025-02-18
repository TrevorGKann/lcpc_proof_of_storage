use crate::databases::{constants, FileMetadata};
use std::env;
use std::path::PathBuf;
use ulid::Ulid;

pub fn get_unencoded_file_handle_from_metadata(file_metadata: &FileMetadata) -> PathBuf {
    // format!("PoS_server_files/{}", file_metadata.id)
    get_unencoded_file_location_from_id(&file_metadata.id_ulid)
}

pub fn get_encoded_file_handle_from_metadata(file_metadata: &FileMetadata) -> PathBuf {
    // format!("PoS_server_files/{}", file_metadata.id)
    get_encoded_file_location_from_id(&file_metadata.id_ulid)
}

pub fn get_merkle_file_handle_from_metadata(file_metadata: &FileMetadata) -> PathBuf {
    // format!("PoS_server_files/{}", file_metadata.id)
    get_merkle_file_location_from_id(&file_metadata.id_ulid)
}

pub fn get_unencoded_file_location_from_id(id: &Ulid) -> PathBuf {
    let mut path = env::current_dir().unwrap();
    path.push(constants::SERVER_FILE_FOLDER);

    //check that directory folder exists
    if !path.exists() {
        std::fs::create_dir_all(&path).unwrap();
    }

    path.push(format!(
        "{}.{}",
        id.to_string(),
        constants::UNENCODED_FILE_EXTENSION
    ));
    path
}

pub fn get_encoded_file_location_from_id(id: &Ulid) -> PathBuf {
    let mut path = env::current_dir().unwrap();
    path.push(constants::SERVER_FILE_FOLDER);

    //check that directory folder exists
    if !path.exists() {
        std::fs::create_dir_all(&path).unwrap();
    }

    path.push(format!(
        "{}.{}",
        id.to_string(),
        constants::ENCODED_FILE_EXTENSION
    ));
    path
}

pub fn get_merkle_file_location_from_id(id: &Ulid) -> PathBuf {
    let mut path = env::current_dir().unwrap();
    path.push(constants::SERVER_FILE_FOLDER);

    //check that directory folder exists
    if !path.exists() {
        std::fs::create_dir_all(&path).unwrap();
    }

    path.push(format!(
        "{}.{}",
        id.to_string(),
        constants::MERKLE_FILE_EXTENSION
    ));
    path
}

pub fn push_server_dir_to_pathbuf(pathbuf: &mut PathBuf) {
    pathbuf.push(constants::SERVER_FILE_FOLDER);

    //check that directory folder exists
    if !pathbuf.exists() {
        std::fs::create_dir_all(&pathbuf).unwrap();
    }
}
