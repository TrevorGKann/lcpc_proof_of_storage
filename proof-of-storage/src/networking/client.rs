use std::cmp::min;

use anyhow::{bail, ensure, Result};
use bitvec::macros::internal::funty::Unsigned;
use bitvec::view::BitViewSized;
use blake3::traits::digest;
use digest::{Digest, FixedOutputReset, Output};
use ff::{Field, PrimeField};
use futures::{SinkExt, StreamExt};
use itertools::Itertools;
use num_traits::{One, ToPrimitive, Zero};
use rand::seq::IteratorRandom;
use rand::thread_rng;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;
use tokio::fs;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{Deserializer, formats::Json, Serializer};

use lcpc_2d::{FieldHash, LcCommit, LcEncoding, LcRoot, open_column, ProverError};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::*;
use crate::databases::*;
use crate::fields::{convert_byte_vec_to_field_elements_vec, convert_field_elements_vec_to_byte_vec, evaluate_field_polynomial_at_point, evaluate_field_polynomial_at_point_with_elevated_degree, writable_ft63};
use crate::fields::writable_ft63::WriteableFt63;
use crate::lcpc_online::{client_verify_commitment, CommitDimensions, CommitOrLeavesOutput, CommitRequestType, convert_file_data_to_commit, FldT, get_PoS_soudness_n_cols, hash_column_to_digest, server_retreive_columns};
use crate::networking::server;
use crate::networking::server::{get_aspect_ratio_default_from_field_len, get_aspect_ratio_default_from_file_len};
use crate::networking::shared::*;

const FIXED_RANDOM_SEED_CHANGE_LATER: u64 = 1337;

// todo: need to not have this connect each time because it'll have to log in each time too. need to keep a constant connection
#[tracing::instrument]
pub async fn upload_file(
    file_name: String,
    num_pre_encoded_columns: Option<usize>,
    num_encoded_columns: Option<usize>,
    server_ip: String,
) -> Result<FileMetadata> {
    use std::path::Path;
    tracing::debug!("reading file {} from disk", file_name);
    let file_path = Path::new(&file_name);
    let mut file_data = fs::read(file_path).await?;
    let mut field_file_data = convert_byte_vec_to_field_elements_vec(&file_data);

    let (num_pre_encoded_columns, num_encoded_columns, required_columns_to_test)
        = match (num_pre_encoded_columns, num_encoded_columns) {
        (Some(num_pre_encoded_columns), Some(num_encoded_columns)) => {
            ensure!(num_pre_encoded_columns >= 1 && num_encoded_columns >= 2,
                "Number of columns and encoded columns must be greater than 0");
            ensure!(num_encoded_columns.is_power_of_two(),
                "Number of encoded columns must be a power of 2");
            ensure!(num_encoded_columns >= 2 * num_pre_encoded_columns,
                "Number of encoded columns must be greater than 2 * number of columns");

            (num_pre_encoded_columns, num_encoded_columns, crate::networking::server::get_soundness_from_matrix_dims(num_pre_encoded_columns, num_encoded_columns))
        }
        (Some(cols), None) => {
            ensure!(cols >= 1, "Number of columns must be greater than 0");

            let rounded_cols = cols.next_power_of_two();
            let enc_cols = (rounded_cols + 1).next_power_of_two();
            (cols, enc_cols, crate::networking::server::get_soundness_from_matrix_dims(cols, enc_cols))
        }
        (None, Some(enc_cols)) => {
            ensure!(enc_cols >= 2, "Number of encoded columns must be greater than 2");
            ensure!(enc_cols.is_power_of_two(), "Number of encoded columns must be a power of 2");

            let cols = enc_cols / 2;
            (cols, enc_cols, crate::networking::server::get_soundness_from_matrix_dims(cols, enc_cols))
        }
        _ => get_aspect_ratio_default_from_file_len::<WriteableFt63>(file_data.len())
    };

    let cols_to_verify = get_columns_from_random_seed(FIXED_RANDOM_SEED_CHANGE_LATER, required_columns_to_test, num_encoded_columns);
    tracing::debug!("pre-computing expected column leaves...");
    let CommitOrLeavesOutput::Leaves(locally_derived_leaves) = convert_file_data_to_commit::<Blake3, WriteableFt63>(
        &field_file_data,
        CommitRequestType::Leaves(cols_to_verify.clone()),
        CommitDimensions::Specified {
            num_pre_encoded_columns,
            num_encoded_columns,
        },
    )? else {
        tracing::error!("Failed to convert file data to leaves");
        bail!("Failed to convert file data to leaves");
    };

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await?;
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    tracing::debug!("sending file to server {}", &server_ip);
    sink.send(ClientMessages::UploadNewFile {
        filename: file_name,
        file: file_data,
        columns: num_pre_encoded_columns,
        encoded_columns: num_encoded_columns,
    }).await?;

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::CompactCommit { mut file_metadata } = transmission else {
        match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed: {}", error);
                bail!("File upload failed: {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response");
            }
        }
    };

    tracing::info!("File upload successful");
    file_metadata.stored_server.server_ip = server_ip.clone().split(":").next().unwrap_or("").to_string();
    file_metadata.stored_server.server_port = server_ip.split(":").nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);

    tracing::info!("client: Requesting a proof from the server...");
    tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
        .await?;

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::Columns { columns: received_columns } = transmission else {
        match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("Proof request failed: {}", error);
                bail!("Proof request failed: {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response");
            }
        }
    };

    let verification_result = client_verify_commitment(
        &file_metadata.root,
        &locally_derived_leaves,
        &cols_to_verify,
        &received_columns,
        get_PoS_soudness_n_cols(&file_metadata));

    if let Err(err) = verification_result {
        tracing::error!("Failed to verify columns: {}", err);
        bail!("Failed to verify columns: {}", err);
        // todo: need to send bad response to server to tell it to delete file
    }
    tracing::info!("File download successful");

    tracing::debug!("adding file metadata to database");
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
    db.create::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, file_metadata.id_ulid.to_string()))
        .content(file_metadata.clone()).await?;

    Ok((file_metadata))
}

pub async fn download_file(file_metadata: FileMetadata,
                           server_ip: String,
                           security_bits: u8,
) -> Result<()>
{
    tracing::info!("requesting file from server");

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::RequestFile { file_metadata: file_metadata.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::File { file: file_data } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed: {}", error);
                bail!("Bad server response {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response");
            }
        }
    };
    tracing::debug!("client: received file from server");
    // verify that file is the correct, commited to file
    // first derive the columns that we'll request from the server
    let column_indices_to_verify = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(&file_metadata),
        file_metadata.num_encoded_columns);
    tracing::trace!("client: requesting the following columns from the server: {:?}", column_indices_to_verify);

    tracing::info!("client: Requesting a proof from the server...");
    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: column_indices_to_verify.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::trace!("Client received: {:?}", transmission);

    let ServerMessages::Columns { columns: received_columns } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed on column retrieval: {}", error);
                bail!("Bad response from server {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response")
            }
        }
    };
    tracing::debug!("client: received columns from server");


    // let locally_derived_leaves_test = get_processed_column_leaves_from_file(&file_metadata, columns_to_verify.clone()).await;
    // let leaves_to_verify
    //     = convert_read_file_to_commit_only_leaves::<Blake3>(&file_data, &columns_to_verify)
    //     .unwrap();
    //
    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);

    let CommitOrLeavesOutput::Leaves(leaves_to_verify) = convert_file_data_to_commit::<Blake3, WriteableFt63>(
        &encoded_file_data,
        CommitRequestType::Leaves(column_indices_to_verify.clone()),
        file_metadata.clone().into(),
    )? else { bail!("Unexpected result from file conversion to Leaves") };

    tracing::debug!("verifying commitment");
    let verification_result = client_verify_commitment(
        &file_metadata.root,
        &leaves_to_verify,
        &column_indices_to_verify,
        &received_columns,
        get_PoS_soudness_n_cols(&file_metadata));

    if verification_result.is_err() {
        tracing::error!("Failed to verify columns");
        //todo send error type to the server (also requires figuring out what to make the server do)
        bail!("failed_to_verify_columns");
    }

    tracing::debug!("client: file verification successful!");
    tracing::debug!("client: writing file to disk at {}", &file_metadata.filename);

    let mut file_handle = File::create(&file_metadata.filename).await?;
    file_handle.write_all(&file_data).await?;

    Ok(())
}


/// this is a thin wrapper for verify_compact_commit function
pub async fn request_proof(
    file_metadata: FileMetadata,
    server_ip: String,
    security_bits: u8,
) -> Result<()> {
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    verify_compact_commit(&file_metadata, &mut stream, &mut sink).await
}

pub fn get_columns_from_random_seed(
    random_seed: u64,
    number_of_columns_to_extract: usize,
    max_column_index: usize,
) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(random_seed);
    let vec_of_all_columns: Vec<usize> = (0..max_column_index).collect();
    let cols_to_verify: Vec<usize> = vec_of_all_columns.iter()
        .map(|col_index| col_index.to_owned())
        .choose_multiple(&mut rng, number_of_columns_to_extract);

    cols_to_verify
}

#[tracing::instrument]
pub async fn verify_compact_commit(
    file_metadata: &FileMetadata,
    stream: &mut SerStream<ServerMessages>,
    sink: &mut DeSink<ClientMessages>,
) -> Result<()> {
    tracing::debug!("sending proof request for {} to server", &file_metadata.filename);

    // pick the random columns
    let cols_to_verify = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER, // todo: refactor s.t. this is a function arg and not hardcoded
        get_PoS_soudness_n_cols(file_metadata),
        file_metadata.num_encoded_columns,
    );

    tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::Columns { columns: received_columns } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed on column retrieval: {}", error);
                bail!("Bad response from server {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response")
            }
        }
    };

    // todo: optimization: the client should extract this upon the first reading of the file as it streams
    //  it out to the server and save it locally; this would prevent having to read the file twice.
    let file_data = tokio::fs::read(&file_metadata.filename).await?;
    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::Leaves::<Blake3, WriteableFt63>(locally_derived_leaves) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Leaves(cols_to_verify.clone()),
        CommitDimensions::Specified {
            num_pre_encoded_columns: file_metadata.num_columns,
            num_encoded_columns: file_metadata.num_encoded_columns,
        },
    )? else { bail!("Unexpected failure to extract leaves from local file.") };

    let verification_result = client_verify_commitment(
        &file_metadata.root,
        &locally_derived_leaves,
        &cols_to_verify,
        &received_columns,
        get_PoS_soudness_n_cols(file_metadata));

    if verification_result.is_err() {
        tracing::error!("Failed to verify columns");
        //todo send error to server
        return todo!();
    }
    tracing::debug!("client: file verification successful!");
    Ok(())
}

pub async fn client_request_and_verify_polynomial<D>(
    file_metadata: &FileMetadata,
    server_ip: String,
) -> Result<()>
where
    D: Digest,
{
    tracing::info!("requesting polynomial evaluation from server");

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    let rng = &mut ChaCha8Rng::seed_from_u64(FIXED_RANDOM_SEED_CHANGE_LATER);
    let evaluation_point = WriteableFt63::random(rng);

    sink.send(ClientMessages::RequestPolynomialEvaluation { file_metadata: file_metadata.clone(), evaluation_point })
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::PolynomialEvaluation { evaluation_result } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed on column retrieval: {}", error);
                bail!("Bad response from server {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response")
            }
        }
    };

    tracing::debug!("client: received polynomial evaluation from server");

    let cols_to_verify = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(file_metadata),
        file_metadata.num_encoded_columns,
    );

    tracing::info!("client: Requesting a proof from the server...");
    tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server")
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::Columns { columns: received_columns } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File upload failed on column retrieval: {}", error);
                bail!("Bad response from server {}", error);
            }
            _ => {
                tracing::error!("Unknown server response");
                bail!("Unknown server response")
            }
        }
    };

    let verification_result = lcpc_online::client_online_verify_column_paths(
        &file_metadata.root,
        &cols_to_verify,
        &received_columns,
    );

    if verification_result.is_err() {
        tracing::error!("Failed to verify columns");
        bail!("Failed to verify columns");
    }

    let local_evaluation_check =
        lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
            &evaluation_point,
            &evaluation_result,
            file_metadata.num_rows,
            file_metadata.num_encoded_columns,
            &cols_to_verify,
            &received_columns,
        );

    if local_evaluation_check.is_err() {
        tracing::error!("Failed to verify polynomial evaluation");
        bail!("failed_to_verify_polynomial_evaluation")
    }

    Ok(())
}

pub async fn reshape_file<D>(
    file_metadata: &FileMetadata,
    server_ip: String,
    security_bits: u8,
    new_pre_encoded_columns: usize,
    new_encoded_columns: usize,
) -> Result<FileMetadata>
where
    D: Digest,
{
    tracing::info!("requesting reshaping of file from server");

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::RequestFileReshape { file_metadata: file_metadata.clone(), new_pre_encoded_columns, new_encoded_columns })
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::CompactCommit {
        file_metadata: new_file_metadata
    } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File reshape failed: {}", error);
                bail!("File reshape failed: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    if (new_file_metadata.num_encoded_columns as usize != new_encoded_columns)
        || (new_file_metadata.num_columns != new_pre_encoded_columns) {
        tracing::error!("Failed to reshape file: requested dimensions not met");
        sink.send(ClientMessages::ReshapeResponse { new_file_metadata, old_file_metadata: file_metadata.clone(), accepted: false })
            .await.expect("Failed to send message to server");
        bail!("Failed to reshape file: requested dimensions not met");
    }

    let mut random_seed = ChaCha8Rng::seed_from_u64(FIXED_RANDOM_SEED_CHANGE_LATER);
    let evaluation_point = WriteableFt63::random(&mut random_seed);

    let requested_original_columns = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(file_metadata),
        file_metadata.num_encoded_columns,
    );
    let requested_new_columns = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(&new_file_metadata),
        new_file_metadata.num_encoded_columns,
    );

    sink.send(ClientMessages::RequestReshapeEvaluation {
        old_file_metadata: file_metadata.clone(),
        new_file_metadata: new_file_metadata.clone(),
        evaluation_point,
        columns_to_expand_original: requested_original_columns.clone(),
        columns_to_expand_new: requested_new_columns.clone(),
    }).await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };

    let ServerMessages::ReshapeEvaluation {
        expected_result,
        original_result_vector,
        original_columns,
        new_result_vector,
        new_columns,
    } = transmission else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File reshape failed: {}", error);
                bail!("File reshape failed: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    let old_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &original_result_vector,
        file_metadata.num_rows,
        file_metadata.num_columns,
        &requested_original_columns,
        &original_columns,
    );

    let new_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &new_result_vector,
        new_file_metadata.num_rows,
        new_file_metadata.num_columns,
        &requested_new_columns,
        &new_columns,
    );

    if old_results.is_err() || new_results.is_err() {
        tracing::error!("Failed to verify polynomial evaluation for reshape");
        tracing::error!("Rejecting reshape");

        sink.send(ClientMessages::ReshapeResponse { old_file_metadata: file_metadata.clone(), new_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");

        bail!("failed_to_verify_polynomial_evaluation")
    }

    let old_results = old_results.unwrap();
    let new_results = new_results.unwrap();

    tracing::debug!("Expected result: {:?}", expected_result.clone());
    tracing::debug!("Old evaluation result: {:?}", old_results.clone());
    tracing::debug!("New evaluation result: {:?}", new_results.clone());

    if old_results.to_u64_array() != new_results.to_u64_array() {
        tracing::error!("Polynomial evaluations mismatched between versions. Rejecting reshape");
        sink.send(ClientMessages::ReshapeResponse { old_file_metadata: file_metadata.clone(), new_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");
        bail!("polynomial evaluations mismatched between versions.")
    }

    sink.send(ClientMessages::ReshapeResponse { old_file_metadata: file_metadata.clone(), new_file_metadata: new_file_metadata.clone(), accepted: true })
        .await.expect("Failed to send message to server");

    { //database scope
        let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
        db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
        let create_result = db.create::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, new_file_metadata.id_ulid.to_string()))
            .content(new_file_metadata.clone())
            .await?;
        tracing::debug!("added new file metadata in database: {}", new_file_metadata);

        let delete_result = db.delete::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, file_metadata.id_ulid.to_string()))
            .await?.unwrap();
        tracing::debug!("Deleted file metadata from database: {}", file_metadata);
    }

    tracing::info!("successfully reshaped file");
    Ok(new_file_metadata)
}


pub async fn delete_file(
    file_metadata: FileMetadata,
    server_ip: String,
) -> Result<()> {
    tracing::info!("requesting deletion of file from server");
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::DeleteFile { file_metadata: file_metadata.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);


    let ServerMessages::FileDeleted { filename } = transmission
    else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File deletion failed: {}", error);
                bail!("File deletion failed on server end: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    tracing::info!("File {} deleted from server", filename);


    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
    let delete_result = db.delete::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, file_metadata.id_ulid.to_string()))
        .await?.unwrap();
    tracing::debug!("Deleted file metadata from database: {}", delete_result);

    Ok(())
}

pub async fn append_to_file(
    file_metadata: FileMetadata,
    server_ip: String,
    security_bits: u8,
    data_to_append: Vec<u8>,
) -> Result<FileMetadata> {
    tracing::info!("requesting append to file from server");
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::AppendToFile { file_metadata: file_metadata.clone(), append_data: data_to_append.clone() })
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::CompactCommit { file_metadata: appended_file_metadata } = transmission
    else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File append failed: {}", error);
                bail!("File append failed on server end: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    // todo: These failures should probably be pushed up into a "cancel transaction" message that the client
    //  can send to the server. I also probably need to restructure the messaging interface so the client can
    //  keep the same interaction with the server throughout functions.
    if (file_metadata.num_columns != appended_file_metadata.num_columns)
        || (file_metadata.num_encoded_columns != appended_file_metadata.num_encoded_columns)
    {
        tracing::error!("File append failed: Size of new commit is invalid");
        sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata, old_file_metadata: appended_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");
        bail!("File append failed: Size of new commit is invalid");
    }
    if (appended_file_metadata.filesize_in_bytes != file_metadata.filesize_in_bytes + data_to_append.len()) {
        tracing::error!("File append failed: Insufficient bytes on new commit");
        sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata, old_file_metadata: appended_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");
        bail!("File append failed: Insufficient bytes on new commit");
    }

    let mut random_seed = ChaCha8Rng::seed_from_u64(FIXED_RANDOM_SEED_CHANGE_LATER);
    let evaluation_point = WriteableFt63::random(&mut random_seed);

    let mut requested_columns = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(&file_metadata),
        file_metadata.num_encoded_columns,
    );

    sink.send(ClientMessages::RequestAppendEvaluation {
        old_file_metadata: file_metadata.clone(),
        new_file_metadata: appended_file_metadata.clone(),
        evaluation_point: evaluation_point.clone(),
        columns_to_expand: requested_columns.clone(),
    }).await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::AppendEvaluation {
        original_result_vector,
        original_columns,
        new_result_vector,
        new_columns,
        edited_unencoded_row,
    } = transmission
    else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File append failed: {}", error);
                bail!("File append failed on server end: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };


    // verify the evaluation of both polynomials is correct, although not yet that the append is correct
    let old_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &original_result_vector,
        file_metadata.num_rows,
        file_metadata.num_columns,
        &requested_columns,
        &original_columns,
    );

    let new_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &new_result_vector,
        appended_file_metadata.num_rows,
        appended_file_metadata.num_columns,
        &requested_columns,
        &new_columns,
    );

    let (Ok(old_results), Ok(new_results)) = (old_results, new_results) else {
        tracing::error!("File append failed: verification failed");
        sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata, old_file_metadata: appended_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");
        bail!("File append failed: verification failed");
    };

    // now verify that the file was correctly appended to;
    // We know what information we appended so we can calculate the expected difference between
    // polynomial evaluations
    let original_polynomial_degree = file_metadata.filesize_in_bytes / (WriteableFt63::CAPACITY / 8) as usize;
    let byte_offset = file_metadata.filesize_in_bytes % (WriteableFt63::CAPACITY / 8) as usize;
    let did_coefficient_change = byte_offset != 0;

    let mut expected_difference_between_evaluations = WriteableFt63::zero();
    let mut byte_difference_between_evaluations = Vec::with_capacity(data_to_append.len() + WriteableFt63::CAPACITY as usize);
    if did_coefficient_change {
        let mut changed_coefficient: WriteableFt63 = edited_unencoded_row[original_polynomial_degree % file_metadata.num_columns].clone();

        let original_coefficient_bytes = convert_field_elements_vec_to_byte_vec(&[changed_coefficient], byte_offset);
        byte_difference_between_evaluations.extend(&original_coefficient_bytes);
        let original_coefficient = convert_byte_vec_to_field_elements_vec(&original_coefficient_bytes);

        if original_coefficient.len() != 1 {
            tracing::error!("Expected only one changed coefficient");
            sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata, old_file_metadata: appended_file_metadata, accepted: false })
                .await.expect("Failed to send message to server");
            bail!("Expected only one changed coefficient");
        }

        expected_difference_between_evaluations -= evaluate_field_polynomial_at_point_with_elevated_degree(
            &original_coefficient,
            &evaluation_point,
            original_polynomial_degree as u64,
        );
    }

    byte_difference_between_evaluations.extend_from_slice(&data_to_append);
    let coefficient_difference_between_evaluations
        = convert_byte_vec_to_field_elements_vec(&byte_difference_between_evaluations);


    expected_difference_between_evaluations += evaluate_field_polynomial_at_point_with_elevated_degree(
        &coefficient_difference_between_evaluations,
        &evaluation_point,
        original_polynomial_degree as u64,
    );


    tracing::debug!("Old results: {:?}", &old_results);
    tracing::debug!("New results: {:?}", &new_results);
    tracing::debug!("Expected difference between evaluations: {:?}", &expected_difference_between_evaluations);
    tracing::debug!("Actual difference between evaluation: {:?}", *&new_results - &old_results);

    if new_results != *&old_results + expected_difference_between_evaluations {
        tracing::error!("File append failed: new results did not match expected results");
        sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata, old_file_metadata: appended_file_metadata, accepted: false })
            .await.expect("Failed to send message to server");
        bail!("File append failed: new results did not match expected results");
    }

    sink.send(ClientMessages::EditOrAppendResponse { new_file_metadata: file_metadata.clone(), old_file_metadata: appended_file_metadata.clone(), accepted: true })
        .await.expect("Failed to send message to server");


    // delete old file in the database and replace with the new one
    let mut db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
    db.create::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, appended_file_metadata.id_ulid.to_string()))
        .content(appended_file_metadata.clone()).await?;
    db.delete::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE, file_metadata.id_ulid.to_string())).await?;

    Ok(appended_file_metadata)
}

pub async fn edit_file(
    file_metadata: FileMetadata,
    server_ip: String,
    security_bits: u8,
    data_to_append: Vec<u8>,
    edit_start_location: usize,
) -> Result<FileMetadata> {
    let data_to_append_len = data_to_append.len();
    ensure!(data_to_append_len + edit_start_location <= file_metadata.filesize_in_bytes,
        "Edited data location will end out of bounds");
    // todo: This doesn't have to be the case, but it is for now.

    tracing::info!("requesting edit to file from server; file: {} @ server: {}", file_metadata.filename, server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::EditFileBytes {
        file_metadata: file_metadata.clone(),
        start_byte: edit_start_location,
        replacement_bytes: data_to_append.clone(),
    }).await.expect("failed to send initial edit message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::CompactCommit { file_metadata: edited_file_metadata } = transmission
    else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File append failed: {}", error);
                bail!("File append failed on server end: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    ensure!(edited_file_metadata.filesize_in_bytes == file_metadata.filesize_in_bytes,
        "file unexpectedly changed shape on edit");
    ensure!(edited_file_metadata.filename == file_metadata.filename,
        "file unexpectedly changed name on edit");
    ensure!(edited_file_metadata.num_rows == file_metadata.num_rows,
        "file unexpectedly changed number of rows on edit");
    ensure!(edited_file_metadata.num_columns == file_metadata.num_columns,
        "file unexpectedly changed number of columns on edit");
    ensure!(edited_file_metadata.num_encoded_columns == file_metadata.num_encoded_columns,
        "file unexpectedly changed number of encoded columns on edit");


    let mut random_seed = ChaCha8Rng::seed_from_u64(FIXED_RANDOM_SEED_CHANGE_LATER);
    let evaluation_point = WriteableFt63::random(&mut random_seed);

    let mut requested_columns = get_columns_from_random_seed(
        FIXED_RANDOM_SEED_CHANGE_LATER,
        get_PoS_soudness_n_cols(&file_metadata),
        file_metadata.num_encoded_columns,
    );


    let first_edited_row = (edit_start_location / file_metadata.num_columns) / (WriteableFt63::CAPACITY as usize / 8);
    let last_edited_row = ((edit_start_location + data_to_append_len) / file_metadata.num_columns) / (WriteableFt63::CAPACITY as usize / 8);
    sink.send(ClientMessages::RequestEditEvaluation {
        old_file_metadata: file_metadata.clone(),
        new_file_metadata: edited_file_metadata.clone(),
        evaluation_point: evaluation_point.clone(),
        columns_to_expand: requested_columns.clone(),
        requested_unencoded_row_range_inclusive: (first_edited_row, last_edited_row),
    }).await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        bail!("Failed to receive message from server");
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::EditEvaluation {
        original_result_vector,
        original_columns,
        new_result_vector,
        new_columns,
        edited_unencoded_rows
    } = transmission
    else {
        return match transmission {
            ServerMessages::ErrorResponse { error } => {
                tracing::error!("File append failed: {}", error);
                bail!("File append failed on server end: {}", error)
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                bail!("Unknown server response: {:?}", transmission)
            }
        }
    };

    ensure!(edited_unencoded_rows.len() >= data_to_append_len, "server sent insufficient data to verify polynomial");
    ensure!(edited_unencoded_rows.len() % (WriteableFt63::BYTE_CAPACITY as usize * file_metadata.num_columns) == 0,
        "server did not send an entire row");


    // verify the evaluation of both polynomials is correct, although not yet that the edit is correct
    let old_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &original_result_vector,
        file_metadata.num_rows,
        file_metadata.num_columns,
        &requested_columns,
        &original_columns,
    );

    let new_results = lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &new_result_vector,
        edited_file_metadata.num_rows,
        edited_file_metadata.num_columns,
        &requested_columns,
        &new_columns,
    );

    let (Ok(old_results), Ok(new_results)) = (old_results, new_results)
    else {
        tracing::error!("File append failed: verification failed");
        sink.send(ClientMessages::EditOrAppendResponse {
            new_file_metadata: file_metadata,
            old_file_metadata: edited_file_metadata,
            accepted: false,
        })
            .await.expect("Failed to send message to server");
        bail!("File append failed: verification failed");
    };

    // Now calculate that the edit is correct. Since we know what data changed we can calculate the
    // expected difference between the polynomial evaluations
    let start_byte_offset_within_coefficient
        = edit_start_location % (WriteableFt63::BYTE_CAPACITY as usize);
    let did_start_coefficient_change = start_byte_offset_within_coefficient != 0;
    // todo: replace all WriteableFt63::CAPACITY / 8 with WriteableFt63::BYTE_CAPACITY
    let start_byte_offset_within_first_edited_row
        = edit_start_location % (WriteableFt63::BYTE_CAPACITY as usize * file_metadata.num_columns);

    let end_byte_offset_within_coefficient = (edit_start_location + data_to_append_len)
        % (WriteableFt63::BYTE_CAPACITY as usize);
    let did_end_coefficient_change = end_byte_offset_within_coefficient != 0;
    let end_byte_offset_within_last_edited_row
        = start_byte_offset_within_first_edited_row + data_to_append_len;
    // note: I actually don't know if I need to do this since the bytes sent back from the server I
    //  know are aligned with the coefficients. I think i can just copy the sent back bytes, splice
    //  the new bytes in then evaluate the expected difference. This is slightly inneficient for the
    //  case where the edit is small and the row is big, but oh well. I think this is the cleanest
    //  way to do it for now
    // optimization: calculate the first and last edited coefficient and only evaluate the
    //  difference between them

    let mut expected_edited_bytes = edited_unencoded_rows.clone();
    expected_edited_bytes.splice(
        start_byte_offset_within_first_edited_row
            ..start_byte_offset_within_first_edited_row + data_to_append_len,
        data_to_append.into_iter());
    ensure!(expected_edited_bytes.len() == edited_unencoded_rows.len(),
        "coefficient bytes unexpectedly changed upon replacement");

    let original_coefficients
        = fields::convert_byte_vec_to_field_elements_vec(&edited_unencoded_rows);
    let new_coefficients
        = fields::convert_byte_vec_to_field_elements_vec(&expected_edited_bytes);
    ensure!(original_coefficients.len() == new_coefficients.len(),
        "num coefficients unexpectedly have different lenghts");

    let changed_row_start_degree = first_edited_row * file_metadata.num_columns;
    let expected_difference = fields::evaluate_field_polynomial_at_point_with_elevated_degree(
        &new_coefficients,
        &evaluation_point,
        changed_row_start_degree as u64,
    ) - fields::evaluate_field_polynomial_at_point_with_elevated_degree(
        &original_coefficients,
        &evaluation_point,
        changed_row_start_degree as u64,
    );

    ensure!(old_results + expected_difference == new_results,
    "file append failed: verification failed");

    sink.send(ClientMessages::EditOrAppendResponse {
        new_file_metadata: file_metadata.clone(),
        old_file_metadata: edited_file_metadata.clone(),
        accepted: true,
    }).await.expect("Failed to send message to server");


    // delete old file in the database and replace with the new one
    let mut db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
    db.create::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE,
                                       edited_file_metadata.id_ulid.to_string()))
        .content(edited_file_metadata.clone()).await?;
    db.delete::<Option<FileMetadata>>((constants::CLIENT_METADATA_TABLE,
                                       file_metadata.id_ulid.to_string())).await?;

    Ok(edited_file_metadata)
}

pub async fn get_client_metadata_from_database_by_filename(filename: &String)
                                                           -> Result<Option<FileMetadata>> {
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::CLIENT_NAMESPACE).use_db(constants::CLIENT_DATABASE_NAME).await?;
    let mut result: Vec<FileMetadata>
        = db.query("SELECT * FROM $table WHERE filename = $filename LIMIT 2")
        .bind(("table", constants::CLIENT_METADATA_TABLE))
        .bind(("file", filename))
        .await?.take(0)?;
    if result.is_empty() {
        Ok(None)
    } else {
        if result.len() > 1 {
            tracing::warn!("Multiple files with the same filename found in database, \
            taking first found instance");
        }
        Ok(Some(result.swap_remove(0)))
    }
}