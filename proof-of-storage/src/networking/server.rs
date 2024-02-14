use blake3::traits::digest;
use serde::{Serialize, Deserialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serde::{formats::Json, Serializer, Deserializer};
use lcpc_2d::{LcCommit, LcEncoding};
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

#[tokio::test]
async fn server_main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .finish();
    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber)?;


    let listener = TcpListener::bind("0.0.0.0:8080").await
        .map_err(|e| tracing::error!("server failed to start: {:?}", e))
        .expect("failed to initialize listener");

    tokio::spawn(async move {
        //server logic

        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move { handle_client_loop(stream).await });
        }
    });
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
            ClientMessages::RequestFile { filename } => {
                handle_client_request_file(filename).await
            }
            ClientMessages::RequestFileRow { filename, row } => {
                handle_client_request_file_row(filename, row).await
            }
            ClientMessages::EditFileRow { filename, row, file } => {
                handle_client_edit_file_row(filename, row, file).await
            }
            ClientMessages::AppendToFile { filename, file } => {
                handle_client_append_to_file(filename, file).await
            }
            ClientMessages::RequestEncodedColumn { filename, row } => {
                handle_client_request_encoded_column(filename, row).await
            }
            ClientMessages::RequestProof { filename, columns_to_verify } => {
                handle_client_request_proof(filename, columns_to_verify).await
            }
            ClientMessages::RequestPolynomialEvaluation { filename, evaluation_point } => {
                handle_client_request_polynomial_evaluation(filename, evaluation_point).await
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

    tokio::fs::write(&filename, file_data).await
        .map_err(|e| tracing::error!("failed to write file: {:?}", e))
        .expect("failed to write file");


    let field_vector = fields::read_file_path_to_field_elements_vec(&filename);

    let data_min_width = (field_vector.len() as f32).sqrt().ceil() as usize;
    let data_realized_width = data_min_width.next_power_of_two();
    let matrix_colums = (data_realized_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&field_vector, &encoding).unwrap();
    let root = commit.get_root();

    CompactCommit { commit}
}

#[tracing::instrument]
async fn handle_client_request_file(filename: String) -> InternalServerMessage {
    // get the requested file from the server
    // send the file to the client
    unimplemented!("handle_client_request_file");
}

#[tracing::instrument]
async fn handle_client_request_file_row(filename: String, row: usize) -> InternalServerMessage {
    // get the requested row from the file
    // send the row to the client
    unimplemented!("handle_client_request_file_row");
}

#[tracing::instrument]
async fn handle_client_edit_file_row(filename: String, row: usize, file: Vec<u8>) -> InternalServerMessage {
    // edit the requested row in the file
    unimplemented!("handle_client_edit_file_row");
}

#[tracing::instrument]
async fn handle_client_append_to_file(filename: String, file: Vec<u8>) -> InternalServerMessage {
    // append the file to the requested file
    unimplemented!("handle_client_append_to_file");
}

#[tracing::instrument]
async fn handle_client_request_encoded_column(filename: String, row: usize) -> InternalServerMessage {
    // get the requested column from the file
    // send the column to the client
    unimplemented!("handle_client_request_encoded_column");
}

#[tracing::instrument]
async fn handle_client_request_proof(filename: String, columns_to_verify: Vec<usize>) -> InternalServerMessage {
    // get the requested proof from the file
    // send the proof to the client
    unimplemented!("handle_client_request_proof");
}

#[tracing::instrument]
async fn handle_client_request_polynomial_evaluation(filename: String, evaluation_point: WriteableFt63) -> InternalServerMessage {
    // get the requested polynomial evaluation from the file
    // send the evaluation to the client
    unimplemented!("handle_client_request_polynomial_evaluation");
}
