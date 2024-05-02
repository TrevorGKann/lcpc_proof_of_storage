use blake3::traits::digest;
use digest::{Digest, FixedOutputReset, Output};
use futures::{SinkExt, StreamExt};
use rand::seq::IteratorRandom;
use rand::thread_rng;
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
use crate::lcpc_online::{client_verify_commitment, get_PoS_soudness_n_cols};
use crate::networking::server;
use crate::networking::shared::*;

//todo need to not have this connect each time because it'll have to log in each time too. need to keep a constant connection
#[tracing::instrument]
pub async fn upload_file(
    file_name: String,
    // rows: usize,
    columns: usize,
    server_ip: String,
) -> Result<(ClientOwnedFileMetadata, PoSRoot), Box<dyn std::error::Error>> {
    use std::path::Path;
    let file_path = Path::new(&file_name);
    let mut file_data = fs::read(file_path).await.unwrap();


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
    stream: &mut SerStream<ServerMessages>,
    sink: &mut DeSink<ClientMessages>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!("sending proof request for {} to server", &file_metadata.filename);

    // pick the random columns
    let mut rng = thread_rng();
    let vec_of_all_columns: Vec<usize> = (0..file_metadata.num_encoded_columns).collect();
    let cols_to_verify: Vec<usize> = vec_of_all_columns.iter()
        .map(|col_index| col_index.to_owned())
        .choose_multiple(&mut rng, get_PoS_soudness_n_cols(file_metadata));

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::debug!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::Columns { columns: received_columns } => {
            let leaves = get_processed_column_leaves_from_file(file_metadata, cols_to_verify.clone()).await;
            let verification_result = client_verify_commitment(
                &file_metadata.root,
                &leaves,
                &cols_to_verify,
                &received_columns,
                get_PoS_soudness_n_cols(file_metadata));

            if verification_result.is_err() {
                tracing::error!("Failed to verify colums");
                //todo return error type
                return todo!();
            }
        }
        _ => {
            tracing::error!("Unexpected server response");
            todo!("add custom error type for client errors")
        }
    }
    Ok(())
}

pub async fn get_processed_column_leaves_from_file(
    file_metadata: &ClientOwnedFileMetadata,
    cols_to_verify: Vec<usize>,
) -> Vec<Output<Blake3>> {
    let (root, commit, file_metadata) = server::convert_file_to_commit(&file_metadata.filename, file_metadata.num_encoded_columns)
        .map_err(|e| {
            tracing::error!("failed to convert file to commit: {:?}", e);
        })
        .expect("failed to convert file to commit");


    let mut leaves = Vec::with_capacity(cols_to_verify.len());
    for col in cols_to_verify {
        leaves.push(commit.hashes[col]);
    }
    leaves
}