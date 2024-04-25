use blake3::traits::digest;
use digest::{Digest, FixedOutputReset, Output};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{Deserializer, formats::Json, Serializer};

use lcpc_2d::{LcCommit, LcEncoding, LcRoot};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::*;
use crate::fields::writable_ft63::WriteableFt63;
use crate::file_metadata::*;
use crate::lcpc_online::get_PoS_soudness_n_cols;
use crate::networking::shared::*;

//todo need to not have this connect each time because it'll have to log in each time too. need to keep a constant connection
#[tracing::instrument]
pub async fn upload_file(
    file_name: String,
    // rows: usize,
    columns: usize,
    server_ip: String,
) -> Result<(ClientOwnedFileMetadata, PoSRoot), Box<dyn std::error::Error>> {
    let mut file_data = fs::read(&file_name).await.unwrap();


    tracing::debug!("reading file {} from disk", file_name);

    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    tracing::debug!("sending file to server {}", &server_ip);
    sink.send(ClientMessages::UploadNewFile { filename: file_name, file: file_data, columns })
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::info!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::CompactCommit { root, file_metadata } => {
            tracing::info!("File upload successful");
            Ok((file_metadata, root))
            //todo need to test that the CompactCommit was succesfull
        }
        ServerMessages::BadResponse { error } => {
            tracing::error!("File upload failed: {}", error);
            Err(Box::from(error))
        }
        _ => {
            tracing::error!("Unknown server response");
            Err(Box::from("Unknown server response"))
        }
    }
}

#[tracing::instrument]
pub async fn verify_compact_commit(
    file_metadata: &ClientOwnedFileMetadata,
    root: &PoSRoot,
    cols_to_verify: Vec<u64>,
    stream: &mut SerStream<ServerMessages>,
    sink: &mut DeSink<ClientMessages>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!("sending proof request for {} to server", &file_metadata.filename);

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::info!("Client received: {:?}", transmission);

    // match transmission {
    //     ServerMessages::
    //     _ => {
    //         tracing::error!("Unknown server response");
    //         Err(Box::from("Unknown server response"))
    //     }
    // }
    todo!()
}