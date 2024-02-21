use std::io::SeekFrom;
use blake3::traits::digest;
use serde::{Serialize, Deserialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_serde::{formats::Json, Serializer, Deserializer};
use lcpc_2d::{LcCommit, LcEncoding, LcRoot};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
use tokio::fs::File;
use crate::networking::shared;
use crate::networking::shared::*;
use blake3::{Hash, Hasher as Blake3};
use futures::{SinkExt, StreamExt};
use crate::fields;
use crate::fields::writable_ft63::WriteableFt63;
use crate::networking::shared::ClientMessages::ClientKeepAlive;
use crate::networking::shared::ServerMessages::*;

type InternalServerMessage = ServerMessages<String>;

#[tracing::instrument]
pub async fn server_main(port: u16, verbosity: u8) -> Result<(), Box<dyn std::error::Error>> {


    let max_level = match verbosity {
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        3 => tracing::Level::TRACE,
        _ => tracing::Level::ERROR,
    };
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_max_level(max_level)
        .finish();
    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber)?;

    tracing::info!("Server starting, verbosity level: {:?}", verbosity);

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


async fn handle_client_loop(mut stream: TcpStream) {
    let (mut stream, mut sink)
        = wrap_stream::<InternalServerMessage, ClientMessages>(stream);

    while let Some(Ok(transmission)) = stream.next().await {
        tracing::info!("Server received: {:?}", transmission);
        let response = match transmission {
            ClientMessages::UserLogin { username, password } => {
                handle_client_user_login(username, password).await
            }
            ClientMessages::UploadNewFile { filename, file, rows, columns } => {
                handle_client_upload_new_file(filename, file, rows, columns).await
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
                handle_client_request_polynomial_evaluation(file_metadata, evaluation_point).await
            }
            _ => { ServerMessages::ServerKeepAlive }
        };


        //todo: remove this and implement another way to send the responses of the internal
        // functions while still possibly being able to send a keep alive message from within
        // the functions
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
async fn handle_client_user_login(username: String, password: String) -> InternalServerMessage {
    // check if the username and password are valid
    // if they are, send a message to the client that the login was successful
    // if they are not, send a message to the client that the login was unsuccessful
    UserLoginResponse { success: true }
    // todo: logic here
}

#[tracing::instrument]
async fn handle_client_upload_new_file(filename: String, file_data: Vec<u8>, rows: usize, columns: usize) -> InternalServerMessage {
    // save the file to the server

    // check if rows and columns are valid first
    if (dims_ok(rows, columns, file_data.len())) {
        return ServerMessages::BadResponse { error: "Invalid rows or columns".to_string() };
    }

    tokio::fs::write(&filename, file_data).await
        .map_err(|e| tracing::error!("failed to write file: {:?}", e))
        .expect("failed to write file");


    // let field_vector = fields::read_file_path_to_field_elements_vec(&filename);
    //
    // let data_min_width = (field_vector.len() as f32).sqrt().ceil() as usize;
    // let data_realized_width = data_min_width.next_power_of_two();
    // let matrix_colums = (data_realized_width + 1).next_power_of_two();
    // let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_colums);
    // let commit = LigeroCommit::<Blake3, _>::commit(&field_vector, &encoding).unwrap();
    // let root = commit.get_root();
    let (root, file_metadata) = convert_file_to_commit(&filename, rows, columns)
        .map_err(|e| {
            tracing::error!("failed to convert file to commit: {:?}", e);
            return make_bad_response(format!("failed to convert file to commit: {:?}", e));
        })
        .expect("failed to convert file to commit");

    CompactCommit { root, file_metadata }
}

#[tracing::instrument]
async fn handle_client_request_file(file_metadata: FileMetadata) -> InternalServerMessage {
    // get the requested file from the server
    // send the file to the client

    if check_file_metadata(&file_metadata).is_err() {
        return make_bad_response("file metadata is invalid".to_string());
    }

    let file = tokio::fs::read(&file_metadata.filename).await
        .map_err(|e| tracing::error!("failed to read file: {:?}", e))
        .expect("failed to read file");

    ServerMessages::File { file }
}

#[tracing::instrument]
async fn handle_client_request_file_row(file_metadata: FileMetadata, row: usize) -> InternalServerMessage {
    // get the requested row from the file
    // send the row to the client

    if check_file_metadata(&file_metadata).is_err() {
        return make_bad_response("file metadata is invalid".to_string());
    }

    let mut file = tokio::fs::File::open(&file_metadata.filename).await
        .map_err(|e| tracing::error!("failed to open file: {:?}", e))
        .expect("failed to open file");

    let seek_pointer = file_metadata.encoded_columns * row;
    file.seek(SeekFrom::Start(seek_pointer as u64));

    let mut file_data = Vec::<u8>::with_capacity(file_metadata.encoded_columns);
    file.take(file_metadata.encoded_columns as u64).read_to_end(&mut file_data);

    ServerMessages::FileRow { row: file_data }
}

#[tracing::instrument]
async fn handle_client_edit_file_row(file_metadata: FileMetadata, row: usize, new_file_data: Vec<u8>) -> InternalServerMessage {
    // edit the requested row in the file

    if check_file_metadata(&file_metadata).is_err() {
        return make_bad_response("file metadata is invalid".to_string());
    }

    if (new_file_data.len() != file_metadata.encoded_columns) {
        return make_bad_response("new file data is not the correct length".to_string());
    }

    //todo need to keep previous file and commit for checking that edit was successful clientside

    { //scope for file opening
        let mut file = tokio::fs::File::open(&file_metadata.filename).await
            .map_err(|e| {
                tracing::error!("failed to open file: {:?}", e);
                return make_bad_response(format!("failed to open file:  {:?}", e));
            })
            .expect("failed to open file");

        let seek_pointer = file_metadata.encoded_columns * row;
        file.seek(SeekFrom::Start(seek_pointer as u64));

        file.write_all(&new_file_data).await
            .map_err(|e| {
                tracing::error!("failed to write to file: {:?}", e);
                return make_bad_response(format!("failed to write to file: {:?}", e));
            })
            .expect("failed to write to file");
    }

    let (root, updated_file_metadata) =
        convert_file_to_commit(&file_metadata.filename, file_metadata.rows, file_metadata.encoded_columns)
        .map_err(|e| {
            tracing::error!("failed to convert file to commit: {:?}", e);
            return make_bad_response(format!("failed to convert file to commit: {:?}", e));
        })
        .expect("failed to convert file to commit");

    CompactCommit { root, file_metadata: updated_file_metadata }
}

#[tracing::instrument]
async fn handle_client_append_to_file(file_metadata: FileMetadata, file_data: Vec<u8>) -> InternalServerMessage {
    // append the file to the requested file

    if check_file_metadata(&file_metadata).is_err() {
        return make_bad_response("file metadata is invalid".to_string());
    }

    { //scope for file opening
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&file_metadata.filename).await
            .map_err(|e| {
                tracing::error!("failed to open file: {:?}", e);
                return make_bad_response(format!("failed to open file:  {:?}", e));
            })
            .expect("failed to open file");

        file.write_all(&file_data).await
            .map_err(|e| {
                tracing::error!("failed to write to file: {:?}", e);
                return make_bad_response(format!("failed to write to file: {:?}", e));
            })
            .expect("failed to write to file");
    }

    let (root, updated_file_metadata) =
        convert_file_to_commit(&file_metadata.filename, file_metadata.rows, file_metadata.encoded_columns)
            .map_err(|e| {
                tracing::error!("failed to convert file to commit: {:?}", e);
                return make_bad_response(format!("failed to convert file to commit: {:?}", e));
            })
            .expect("failed to convert file to commit");

    CompactCommit { root, file_metadata: updated_file_metadata }
}

#[tracing::instrument]
async fn handle_client_request_encoded_column(file_metadata: FileMetadata, row: usize) -> InternalServerMessage {
    // get the requested column from the file
    // send the column to the client

    if check_file_metadata(&file_metadata).is_err() {
        return make_bad_response("file metadata is invalid".to_string());
    }
    unimplemented!("handle_client_request_encoded_column");
}

#[tracing::instrument]
async fn handle_client_request_proof(file_metadata: FileMetadata, columns_to_verify: Vec<usize>) -> InternalServerMessage {
    // get the requested proof from the file
    // send the proof to the client
    unimplemented!("handle_client_request_proof");
}

#[tracing::instrument]
async fn handle_client_request_polynomial_evaluation(file_metadata: FileMetadata, evaluation_point: WriteableFt63) -> InternalServerMessage {
    // get the requested polynomial evaluation from the file
    // send the evaluation to the client
    unimplemented!("handle_client_request_polynomial_evaluation");
}

fn convert_file_to_commit(filename: &str, requested_rows: usize, requested_columns: usize) -> Result<(LcRoot<Blake3, LigeroEncoding<WriteableFt63>>, FileMetadata), Box<dyn std::error::Error>> {
    //todo need logic to request certain columns and row values
    let field_vector = fields::read_file_path_to_field_elements_vec(filename);
    let data_min_width = (field_vector.len() as f32).sqrt().ceil() as usize;
    let data_realized_width = data_min_width.next_power_of_two();
    let matrix_colums = (data_realized_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&field_vector, &encoding).unwrap();
    let root = commit.get_root();

    let file_metadata = FileMetadata {
        filename: filename.to_string(),
        rows: field_vector.len() / matrix_colums + 1,
        encoded_columns: matrix_colums,
        filesize_in_bytes: field_vector.len(),
    };
    Ok((root, file_metadata))
}

fn make_bad_response(message: String) -> InternalServerMessage {
    BadResponse { error: message }
}

fn dims_ok(rows: usize, columns: usize, file_size: usize) -> bool {
    let total_size = rows * columns / 2 >= file_size;
    let col_power_2 = columns.is_power_of_two();
    total_size && col_power_2
}

fn check_file_metadata(file_metadata: &FileMetadata) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}