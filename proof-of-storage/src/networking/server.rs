use std::cmp::min;
use std::io::SeekFrom;

use anyhow::{bail, ensure, Result};
use blake3::{Hash, Hasher as Blake3};
use blake3::traits::digest;
use blake3::traits::digest::Output;
use futures::{SinkExt, StreamExt, TryFutureExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::Mem;
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{Deserializer, formats::Json, Serializer};

use lcpc_2d::{LcCommit, LcEncoding, LcRoot};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::{fields, PoSColumn, PoSCommit};
use crate::fields::{evaluate_field_polynomial_at_point, is_power_of_two, read_file_to_field_elements_vec};
use crate::fields::writable_ft63::WriteableFt63;
use crate::file_metadata::*;
use crate::lcpc_online::{form_side_vectors_for_polynomial_evaluation_from_point, get_PoS_soudness_n_cols, server_retreive_columns, verifiable_polynomial_evaluation};
use crate::networking::shared;
use crate::networking::shared::ClientMessages;
use crate::networking::shared::ServerMessages;
use crate::networking::shared::wrap_stream;

type InternalServerMessage = ServerMessages;

// static DB: Lazy<Surreal<Client>> = Lazy::new(Surreal::init);

#[tracing::instrument]
pub async fn server_main(port: u16, verbosity: u8) -> Result<()> {
    tracing::info!("Server starting, verbosity level: {:?}", verbosity);

    // tracing::debug!("Connecting to local database");
    // DB.connect::<RocksDb>("server_database").await?;
    // DB.use_ns("Server").use_db("Server").await?;


    tracing::debug!("Binding to port {:?}", port);
    let listening_address = format!("0.0.0.0:{}", port);

    let listener = TcpListener::bind(listening_address).await
        .map_err(|e| tracing::error!("server failed to start: {:?}", e))
        .expect("failed to initialize listener");

    tracing::info!("Server started on port {:?}", listener.local_addr().unwrap());

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(async move { handle_client_loop(stream).await });
    }
    //
    //
    // // client logic
    // let stream = TcpStream::connect("0.0.0.0:8080").await?;
    // let (mut stream, mut sink) = wrap_stream::<ClientMessages,ServerMessages<String>>(stream);
    //
    // sink.send(ClientMessages::UserLogin {username: "trevor".to_owned(), password: "password".to_owned()})
    //     .await.expect("Failed to send message to server");
    //
    // let Some(Ok(transmission)) = stream.next().await else {
    //     tracing::error!("Failed to receive message from server");
    //     return core::result::Result::Err(Box::from("Failed to receive message from server"));
    // };
    // tracing::info!("Client received: {:?}", transmission);
    //
    //
    // sink.send(ClientMessages::RequestEncodedColumn {filename: "test".to_owned(), row: 0})
    //     .await.expect("Failed to send message to server");
    //
    // let Some(Ok(transmission)) = stream.next().await else {
    //     tracing::error!("Failed to receive message from server");
    //     return core::result::Result::Err(Box::from("Failed to receive message from server"));
    // };
    // tracing::info!("Client received: {:?}", transmission);
    //
    Ok(())
}


pub(crate) async fn handle_client_loop(mut stream: TcpStream) {
    let (mut stream, mut sink)
        = wrap_stream::<InternalServerMessage, ClientMessages>(stream);

    while let Some(Ok(transmission)) = stream.next().await {
        tracing::info!("Server received: {:?}", transmission);
        let request_result = match transmission {
            ClientMessages::UserLogin { username, password } => {
                handle_client_user_login(username, password).await
            }
            ClientMessages::UploadNewFile { filename, file, columns, encoded_columns } => {
                handle_client_upload_new_file(filename, file, columns, encoded_columns).await
            }
            ClientMessages::RequestFile { file_metadata } => {
                handle_client_request_file(file_metadata).await
            }
            ClientMessages::RequestFileRow { file_metadata, row } => {
                handle_client_request_file_row(file_metadata, row).await
            }
            ClientMessages::EditFileRow { file_metadata, row, file } => {
                handle_client_edit_file_row(file_metadata, row, file).await
            }
            ClientMessages::AppendToFile { file_metadata, file } => {
                handle_client_append_to_file(file_metadata, file).await
            }
            ClientMessages::RequestEncodedColumn { file_metadata, row } => {
                handle_client_request_encoded_column(file_metadata, row).await
            }
            ClientMessages::RequestProof { file_metadata, columns_to_verify } => {
                handle_client_request_proof(file_metadata, columns_to_verify).await
            }
            ClientMessages::RequestPolynomialEvaluation { file_metadata, evaluation_point } => {
                handle_client_request_polynomial_evaluation(file_metadata, &evaluation_point).await
            }
            ClientMessages::RequestFileReshape { file_metadata, new_pre_encoded_columns, new_encoded_columns } => {
                handle_client_request_file_reshape(file_metadata, new_pre_encoded_columns, new_encoded_columns).await
            }
            ClientMessages::RequestReshapeEvaluation { old_file_metadata, new_file_metadata, evaluation_point, columns_to_expand_original, columns_to_expand_new } => {
                handle_client_request_reshape_evaluation(&old_file_metadata,
                                                         &new_file_metadata,
                                                         &evaluation_point,
                                                         &columns_to_expand_original,
                                                         &columns_to_expand_new).await
            }
            ClientMessages::ReshapeResponse { new_file_metadata, old_file_metadata, accepted } => {
                handle_client_reshape_response(&old_file_metadata, &new_file_metadata, accepted).await
            }
            ClientMessages::DeleteFile { file_metadata } => {
                handle_client_delete_file(file_metadata).await
            }
            ClientMessages::ClientKeepAlive => { Ok(ServerMessages::ServerKeepAlive) }
        };

        let response = request_result.unwrap_or_else(|error| make_bad_response(format!("failed to convert file to commit: {:?}", error)));


        //todo: remove this and implement another way to send the responses of the internal
        // functions while still possibly being able to send a keep alive message from within
        // the functions depending on how long each interaction takes
        sink.send(response)
            .await
            .expect("Failed to send message to client");
    }
}

/* Copilot should ignore this section:
    old server loop code:
    while let Some(Ok(transmission)) = stream.next().await {
        tracing::info!("Server received: {:?}", transmission);
        match transmission {
            ClientMessages::UserLogin { username, password } => {
                sink.send(ServerMessages::UserLoginResponse { success: true })
                    .await
                    .expect("Failed to send message to client");
            }
            ClientMessages::RequestEncodedColumn {filename, row} => {
                sink.send(ServerMessages::EncodedColumn {row: vec![WriteableFt63::from_u64_array([1u64]).expect("AAA")]})
                    .await
                    .expect("Failed to send message to client");
            }
            _ => {}
        }
    }
 */

#[tracing::instrument]
async fn handle_client_user_login(username: String, password: String) -> Result<InternalServerMessage> {
    // check if the username and password are valid
    // if they are, send a message to the client that the login was successful
    // if they are not, send a message to the client that the login was unsuccessful
    Ok(ServerMessages::UserLoginResponse { success: true })
    // todo: logic here
}

/// save the file to the server
#[tracing::instrument]
async fn handle_client_upload_new_file(filename: String,
                                       file_data: Vec<u8>,
                                       pre_encoded_columns: usize,
                                       encoded_columns: usize) //todo: need to use both pre and post encoded columns
                                       -> Result<InternalServerMessage>
{
    // parse the filename to remove all leading slashes
    use std::path::Path;
    use crate::lcpc_online;
    let filename = Path::new(&filename)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();

    if !(is_server_filename_unique(&"server_file_database".to_string(), filename.to_string()).await.expect("Failed to check if filename is unique")) {
        tracing::error!("Server: Client requested a filename that already exists: {}", filename);
        bail!("filename already exists on server, try again with a different one".to_string());
    }

    // check if rows and columns are valid first
    ensure!(lcpc_online::dims_ok(pre_encoded_columns, encoded_columns), "Invalid rows or columns");

    tokio::fs::write(&filename, file_data).await
        .map_err(|e| tracing::error!("failed to write file: {:?}", e))
        .expect("failed to write file"); //todo probably shouldn't be a panic here


    let (root, commit, file_metadata)
        = convert_file_to_commit_internal(filename, Some(pre_encoded_columns))?;

    tracing::info!("server: appending file metadata to database");
    let server_metadata = ServerOwnedFileMetadata {
        filename: filename.to_string(),
        owner: "".to_string(), //todo add users
        commitment: commit.clone(),
    };
    append_server_file_metadata_to_database("server_file_database".to_string(), server_metadata).await;
    //optimize: probably should have a tokio::spawn here in case of colliding writes.
    // in general this needs to be multi-thread safe.

    Ok(ServerMessages::CompactCommit { root, file_metadata })
}

#[tracing::instrument]
async fn handle_client_request_file(file_metadata: ClientOwnedFileMetadata) -> Result<InternalServerMessage> {
    // get the requested file from the server
    // send the file to the client

    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata)?;

    let file = tokio::fs::read(&file_handle.as_str()).await?;

    Ok(ServerMessages::File { file })
}

#[tracing::instrument]
async fn handle_client_request_file_row(file_metadata: ClientOwnedFileMetadata, row: usize) -> Result<InternalServerMessage> {
    // get the requested row from the file
    // send the row to the client

    if check_file_metadata(&file_metadata).is_err() {
        bail!("file metadata is invalid".to_string());
    }

    let mut file = tokio::fs::File::open(&file_metadata.filename).await?;

    let seek_pointer = file_metadata.num_encoded_columns * row;
    file.seek(SeekFrom::Start(seek_pointer as u64));

    let mut file_data = Vec::<u8>::with_capacity(file_metadata.num_encoded_columns);
    file.take(file_metadata.num_encoded_columns as u64).read_to_end(&mut file_data);

    Ok(ServerMessages::FileRow { row: file_data })
}

#[tracing::instrument]
async fn handle_client_edit_file_row(file_metadata: ClientOwnedFileMetadata, row: usize, new_file_data: Vec<u8>) -> Result<InternalServerMessage> {
    // edit the requested row in the file

    check_file_metadata(&file_metadata)?;

    if (new_file_data.len() != file_metadata.num_encoded_columns) {
        bail!("new file data is not the correct length".to_string());
    }

    //todo need to keep previous file and commit for clientside checking that edit was successful

    { //scope for file opening
        let mut file = tokio::fs::File::open(&file_metadata.filename).await?;

        let seek_pointer = file_metadata.num_encoded_columns * row;
        file.seek(SeekFrom::Start(seek_pointer as u64));

        file.write_all(&new_file_data).await?;
    }

    let (root, commit, updated_file_metadata) =
        convert_file_to_commit_internal(&file_metadata.filename, Some(file_metadata.num_columns))?;

    Ok(ServerMessages::CompactCommit { root, file_metadata: updated_file_metadata })
}

#[tracing::instrument]
async fn handle_client_append_to_file(file_metadata: ClientOwnedFileMetadata, file_data: Vec<u8>) -> Result<InternalServerMessage> {
    // append the file to the requested file

    check_file_metadata(&file_metadata)?;

    { //scope for file opening
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&file_metadata.filename).await?;

        file.write_all(&file_data).await?;
    }

    let (root, commit, updated_file_metadata) =
        convert_file_to_commit_internal(&file_metadata.filename, Some(file_metadata.num_columns))?;

    Ok(ServerMessages::CompactCommit { root, file_metadata: updated_file_metadata })
}

#[tracing::instrument]
async fn handle_client_request_encoded_column(file_metadata: ClientOwnedFileMetadata, row: usize) -> Result<InternalServerMessage> {
    // get the requested column from the file
    // send the column to the client

    check_file_metadata(&file_metadata)?;

    unimplemented!("handle_client_request_encoded_column");
}

#[tracing::instrument]
async fn handle_client_request_proof(file_metadata: ClientOwnedFileMetadata, requested_columns: Vec<usize>) -> Result<InternalServerMessage> {
    // get the requested proof from the file
    // send the proof to the client

    tracing::trace!("server: requested columns for client proof: {:?}", requested_columns);
    check_file_metadata(&file_metadata)?;

    let (_, commit) = convert_file_metadata_to_commit(&file_metadata)?;

    let column_collection = server_retreive_columns(&commit, &requested_columns);

    for column in column_collection.iter().take(5) {
        tracing::trace!("server: sending leaf to client: {:x}", column.path[0]);
    }

    Ok(ServerMessages::Columns { columns: column_collection })
}

#[tracing::instrument]
async fn handle_client_request_polynomial_evaluation(file_metadata: ClientOwnedFileMetadata, evaluation_point: &WriteableFt63) -> Result<InternalServerMessage> {
    // get the requested polynomial evaluation from the file
    // send the evaluation to the client
    tracing::info!("server: client requested polynomial evaluation of {:?} at {:?}", &file_metadata.filename, evaluation_point);
    check_file_metadata(&file_metadata)?;

    let (_, commit) = convert_file_metadata_to_commit(&file_metadata)?;

    let (evaluation_left_vector, _)
        = form_side_vectors_for_polynomial_evaluation_from_point(evaluation_point, commit.n_rows, commit.n_cols);


    let result_vector = verifiable_polynomial_evaluation(&commit, &evaluation_left_vector);


    Ok(ServerMessages::PolynomialEvaluation { evaluation_result: result_vector })

    // unimplemented!("handle_client_request_polynomial_evaluation");
}

#[tracing::instrument]
async fn handle_client_delete_file(
    file_metadata: ClientOwnedFileMetadata,
) -> Result<ServerMessages> {
    tracing::info!("server: client requested to delete file: {}", file_metadata.filename);
    let server_side_filename = get_file_handle_from_metadata(&file_metadata).unwrap();
    let os_delete_result = std::fs::remove_file(server_side_filename);

    let db_delete_result = remove_server_file_metadata_from_database_by_filename(
        "server_file_database".to_string(),
        file_metadata.filename.clone(),
    ).await;

    let mut return_string = "".to_string();

    if os_delete_result.is_err() {
        return_string.push_str(format!("error deleting file: {}; ", os_delete_result.unwrap_err()).as_str());
    }
    if !db_delete_result.is_some() {
        return_string.push_str("error deleting file metadata from local database".to_string().as_str());
    }

    if return_string.len() > 0 {
        bail!(return_string);
    }

    Ok(ServerMessages::FileDeleted {
        filename: file_metadata.filename,
    })
}

#[tracing::instrument]
async fn handle_client_request_file_reshape(
    file_metadata: ClientOwnedFileMetadata,
    new_pre_encoded_columns: usize,
    new_encoded_columns: usize,
) -> Result<ServerMessages> {
    let mut updated_file_metadata = file_metadata.clone();
    updated_file_metadata.num_columns = new_pre_encoded_columns;
    updated_file_metadata.num_encoded_columns = new_encoded_columns;

    let (_, updated_commit) = convert_file_metadata_to_commit(&updated_file_metadata)?;
    updated_file_metadata.root = updated_commit.get_root();

    Ok(ServerMessages::CompactCommit {
        root: updated_commit.get_root(),
        file_metadata: updated_file_metadata,
    })
}

#[tracing::instrument]
async fn handle_client_request_reshape_evaluation(
    old_file_metadata: &ClientOwnedFileMetadata,
    new_file_metadata: &ClientOwnedFileMetadata,
    evaluation_point: &WriteableFt63,
    columns_to_expand_original: &Vec<usize>,
    columns_to_expand_new: &Vec<usize>,
) -> Result<ServerMessages> {
    let (_, old_commit) = convert_file_metadata_to_commit(&old_file_metadata)?;
    let (_, new_commit) = convert_file_metadata_to_commit(&new_file_metadata)?;


    let (evaluation_left_vector_for_old_commit, _)
        = form_side_vectors_for_polynomial_evaluation_from_point(evaluation_point, old_commit.n_rows, old_commit.n_cols);
    let result_vector_for_old_commit = verifiable_polynomial_evaluation(&old_commit, &evaluation_left_vector_for_old_commit);
    let old_result = verifiable_polynomial_evaluation(&old_commit, &result_vector_for_old_commit);
    let columns_for_old_commit = server_retreive_columns(&old_commit, &columns_to_expand_original);


    let (evaluation_left_vector_for_new_commit, _)
        = form_side_vectors_for_polynomial_evaluation_from_point(evaluation_point, new_commit.n_rows, new_commit.n_cols);
    let result_vector_for_new_commit = verifiable_polynomial_evaluation(&new_commit, &evaluation_left_vector_for_new_commit);
    let columns_for_new_commit = server_retreive_columns(&new_commit, &columns_to_expand_new);
    let new_result = verifiable_polynomial_evaluation(&new_commit, &result_vector_for_new_commit);


    Ok(ServerMessages::ReshapeEvaluation {
        original_result_vector: result_vector_for_old_commit,
        original_columns: columns_for_old_commit,
        new_result_vector: result_vector_for_new_commit,
        new_columns: columns_for_new_commit,
    })
}

#[tracing::instrument]
async fn handle_client_reshape_response(
    old_file_metadata: &ClientOwnedFileMetadata,
    new_file_metadata: &ClientOwnedFileMetadata,
    accepted: bool,
) -> Result<ServerMessages> {
    if accepted { todo!() } else {
        // I don't think I have to do anything if it's rejected
        todo!()
    }
}


#[tracing::instrument]
pub fn get_aspect_ratio_default_from_field_len(field_len: usize) -> (usize, usize, usize) {
    tracing::debug!("field_len: {}", field_len);

    let data_min_width = (field_len as f32).sqrt().ceil() as usize;
    let num_pre_encoded_columns = if is_power_of_two(data_min_width) {
        data_min_width
    } else {
        data_min_width.next_power_of_two()
    };
    let num_encoded_matrix_columns = (num_pre_encoded_columns + 1).next_power_of_two();


    let soundness = get_soundness_from_matrix_dims(num_pre_encoded_columns, num_encoded_matrix_columns);

    (num_pre_encoded_columns, num_encoded_matrix_columns, soundness)
}

#[tracing::instrument]
pub(crate) fn get_soundness_from_matrix_dims(pre_encoded_cols: usize, encoded_cols: usize) -> usize {
    let soundness_denominator: f64 = ((1f64 + (pre_encoded_cols as f64 / encoded_cols as f64)) / 2f64).log2();
    let theoretical_min = (-128f64 / soundness_denominator).ceil() as usize;
    // can't test more than the number of total columns, sometimes theoretical min is above that.
    // Testing all columns is sufficient for perfect soundness
    min(theoretical_min, encoded_cols)
}

#[tracing::instrument]
pub fn get_aspect_ratio_default_from_file_len<Field: ff::PrimeField>(file_len: usize) -> (usize, usize, usize) {
    tracing::debug!("file_len: {}", file_len);
    let write_out_byte_width = (Field::CAPACITY / 8) as usize;
    let field_len = usize::div_ceil(file_len, write_out_byte_width);
    tracing::debug!("field_len: {}", field_len);

    ///num_pre_encoded_columns, num_encoded_matrix_columns, soundness
    get_aspect_ratio_default_from_field_len(field_len)
}


#[tracing::instrument]
/// a thin wrapper for convert_file_to_commit_internal that should be used with ClientOwnedFileMetadata
pub fn convert_file_metadata_to_commit(file_metadata: &ClientOwnedFileMetadata)
                                       -> Result<(LcRoot<Blake3, LigeroEncoding<WriteableFt63>>, PoSCommit)>
{
    let server_side_filename = get_file_handle_from_metadata(&file_metadata)?;
    let (result_root, result_commit, _)
        = convert_file_to_commit_internal(server_side_filename.as_str(), Some(file_metadata.num_columns))?;
    Ok((result_root, result_commit))
}

#[tracing::instrument]
pub fn convert_file_to_commit_internal(filename: &str, requested_pre_encoded_columns: Option<usize>)
                                       -> Result<(LcRoot<Blake3, LigeroEncoding<WriteableFt63>>, PoSCommit, ClientOwnedFileMetadata)>
{
    // read entire file (we can't get around this)
    // todo: use mem maps so this can be streamed
    tracing::info!("reading file {}", filename);
    let mut file = std::fs::File::open(filename).unwrap();
    let (size_in_bytes, field_vector) = read_file_to_field_elements_vec(&mut file);
    tracing::info!("field_vector: {:?}", field_vector.len());

    ensure!(field_vector.len() > 0, "file is empty");

    let (pre_encoded_columns, encoded_columns, soundness) =
        if requested_pre_encoded_columns.is_none() {
            get_aspect_ratio_default_from_field_len(field_vector.len())
        } else {
            let requested_columns = requested_pre_encoded_columns.unwrap();
            let pre_encoded_columns = requested_columns;
            // encoded columns should be at least 2x pre_encoded columns and needs to be a square
            let encoded_columns = if is_power_of_two(pre_encoded_columns) {
                (pre_encoded_columns + 1).next_power_of_two()
            } else {
                (pre_encoded_columns * 2).next_power_of_two()
            };
            let soundness = get_soundness_from_matrix_dims(pre_encoded_columns, encoded_columns);
            (pre_encoded_columns, encoded_columns, soundness)
        };

    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(pre_encoded_columns, encoded_columns);
    let commit = LigeroCommit::<Blake3, _>::commit(&field_vector, &encoding).unwrap();
    let root = commit.get_root();

    let file_metadata = ClientOwnedFileMetadata {
        stored_server: ServerHost {
            //the client will change this, so this doesn't matter
            server_name: None,
            server_ip: "".to_string(),
            server_port: 0,
        },
        filename: filename.to_string(),
        num_rows: usize::div_ceil(field_vector.len(), pre_encoded_columns),
        num_columns: pre_encoded_columns,
        num_encoded_columns: encoded_columns,
        filesize_in_bytes: size_in_bytes,
        root: root.clone(),
    };

    Ok((root, commit, file_metadata))
}

fn make_bad_response(message: String) -> InternalServerMessage {
    tracing::error!("{}", message);
    ServerMessages::BadResponse { error: message }
}

fn check_file_metadata(file_metadata: &ClientOwnedFileMetadata) -> Result<()> {
    // todo!()
    Ok(())
}

fn get_file_handle_from_metadata(file_metadata: &ClientOwnedFileMetadata) -> Result<String> {
    // todo need additional logic to search database once database is implemented.
    Ok(file_metadata.filename.clone())
}