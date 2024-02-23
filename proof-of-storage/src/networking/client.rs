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

use crate::fields::writable_ft63::WriteableFt63;
use crate::File_Metadata::*;
use crate::networking::shared::*;

#[tracing::instrument]
pub async fn upload_file(file_name: String, rows: usize, columns: usize, server_ip: String) -> Result<(FileMetadata, PosRoot), Box<dyn std::error::Error>> {
    let mut file_data = fs::read(&file_name).await.unwrap();


    tracing::debug!("reading file {} from disk", file_name);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages<String>>(stream);

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