use bitvec::macros::internal::funty::Unsigned;
use bitvec::view::BitViewSized;
use blake3::traits::digest;
use digest::{Digest, FixedOutputReset, Output};
use ff::Field;
use futures::{SinkExt, StreamExt};
use itertools::Itertools;
use num_traits::{One, ToPrimitive, Zero};
use pretty_assertions::assert_eq;
use rand::seq::IteratorRandom;
use rand::thread_rng;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{Deserializer, formats::Json, Serializer};

use lcpc_2d::{FieldHash, LcCommit, LcEncoding, LcRoot, open_column, ProverError};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::*;
use crate::fields::{evaluate_field_polynomial_at_point, writable_ft63};
use crate::fields::writable_ft63::WriteableFt63;
use crate::file_metadata::*;
use crate::lcpc_online::{client_verify_commitment, FldT, get_PoS_soudness_n_cols, hash_column_to_digest, server_retreive_columns};
use crate::networking::server;
use crate::networking::server::{get_aspect_ratio_default_from_field_len, get_aspect_ratio_default_from_file_len};
use crate::networking::shared::*;

//todo need to not have this connect each time because it'll have to log in each time too. need to keep a constant connection
#[tracing::instrument]
pub async fn upload_file(
    file_name: String,
    // rows: usize,
    columns: Option<usize>,
    encoded_columns: Option<usize>,
    server_ip: String,
) -> Result<(ClientOwnedFileMetadata, PoSRoot), Box<dyn std::error::Error>> {
    use std::path::Path;
    tracing::debug!("reading file {} from disk", file_name);
    let file_path = Path::new(&file_name);
    let mut file_data = fs::read(file_path).await.unwrap();

    let (num_pre_encoded_rows, num_encoded_columns, required_columns_to_test)
        = match (columns, encoded_columns) {
        (Some(cols), Some(enc_cols)) => {
            if cols >= enc_cols {
                return Err(Box::from("Number of columns must be greater than or equal to number of encoded columns"));
            }
            if cols < 1 || enc_cols < 2 {
                return Err(Box::from("Number of columns and encoded columns must be greater than 0"));
            }
            if !enc_cols.is_power_of_two() {
                return Err(Box::from("Number of encoded columns must be a power of 2"));
            }
            if !(enc_cols >= 2 * cols) {
                return Err(Box::from("Number of encoded columns must be greater than 2 * number of columns"));
            }
            (cols, enc_cols, crate::networking::server::get_soundness_from_matrix_dims(cols, enc_cols))
        }
        (Some(cols), None) => {
            if cols < 1 {
                return Err(Box::from("Number of columns must be greater than 0"));
            }
            let rounded_cols = if (cols.is_power_of_two()) { cols } else { cols.next_power_of_two() };
            let enc_cols = rounded_cols.next_power_of_two();
            (cols, enc_cols, crate::networking::server::get_soundness_from_matrix_dims(cols, enc_cols))
        }
        (None, Some(enc_cols)) => {
            if enc_cols < 2 {
                return Err(Box::from("Number of columns must be greater than 1, and therefore enc_cols > 2"));
            }
            if !enc_cols.is_power_of_two() {
                return Err(Box::from("Number of encoded columns must be a power of 2"));
            }
            let cols = enc_cols / 2;
            (cols, enc_cols, crate::networking::server::get_soundness_from_matrix_dims(cols, enc_cols))
        }
        _ => get_aspect_ratio_default_from_file_len::<WriteableFt63>(file_data.len())
    };


    let cols_to_verify = get_columns_from_random_seed(1337, required_columns_to_test, num_encoded_columns);
    tracing::debug!("pre-computing expected column leaves...");
    let locally_derived_leaves = convert_read_file_to_commit_only_leaves::<Blake3>(&file_data, &cols_to_verify).unwrap();


    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);


    tracing::debug!("sending file to server {}", &server_ip);
    sink.send(ClientMessages::UploadNewFile {
        filename: file_name,
        file: file_data,
        columns: num_pre_encoded_rows,
        encoded_columns: num_encoded_columns,
    }).await.expect("Failed to send message to server");

    let Some(Ok(mut transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::info!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::CompactCommit { root, mut file_metadata } => {
            tracing::info!("File upload successful");
            //update file_metadata's host
            file_metadata.stored_server.server_ip = server_ip.clone().split(":").collect::<Vec<&str>>()[0].to_string();
            file_metadata.stored_server.server_port = server_ip.split(":").collect::<Vec<&str>>()[1].parse().unwrap();


            tracing::info!("client: Requesting a proof from the server...");
            tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

            sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
                .await.expect("Failed to send message to server");


            let Some(Ok(transmission)) = stream.next().await else {
                tracing::error!("Failed to receive message from server");
                return Err(Box::from("Failed to receive message from server"));
            };
            tracing::debug!("Client received: {:?}", transmission);

            match transmission {
                ServerMessages::Columns { columns: received_columns } => {
                    let locally_derived_leaves_test = get_processed_column_leaves_from_file(&file_metadata, cols_to_verify.clone()).await;
                    assert_eq!(locally_derived_leaves, locally_derived_leaves_test);

                    let verification_result = client_verify_commitment(
                        &file_metadata.root,
                        &locally_derived_leaves,
                        &cols_to_verify,
                        &received_columns,
                        get_PoS_soudness_n_cols(&file_metadata));

                    if verification_result.is_err() {
                        tracing::error!("Failed to verify colums");
                        //todo return error type
                        return Err(Box::from("failed_to_verify_columns".to_string()));
                    }
                    Ok((file_metadata, root))
                }
                _ => {
                    tracing::error!("Unexpected server response");
                    return Err(Box::from("Unknown server response"));
                }
            }
        }
        ServerMessages::BadResponse { error } => {
            tracing::error!("File upload failed: {}", error);
            return Err(Box::from(error));
        }
        _ => {
            tracing::error!("Unknown server response");
            return Err(Box::from("Unknown server response"));
        }
    }
}

pub async fn download_file(file_metadata: ClientOwnedFileMetadata,
                           server_ip: String,
                           security_bits: u8,
) -> Result<(), Box<dyn std::error::Error>>
{
    tracing::info!("requesting file from server");

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    sink.send(ClientMessages::RequestFile { file_metadata: file_metadata.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::debug!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::File { file: file_data } => {
            tracing::debug!("client: received file from server");
            // verify that file is the correct, commited to file
            // first derive the columns that we'll request from the server
            let columns_to_verify = get_columns_from_random_seed(
                1337,
                get_PoS_soudness_n_cols(&file_metadata),
                file_metadata.num_encoded_columns);
            tracing::trace!("client: requesting the following columns from the server: {:?}", columns_to_verify);

            let leaves_to_verify
                = convert_read_file_to_commit_only_leaves::<Blake3>(&file_data, &columns_to_verify)
                .unwrap();

            tracing::info!("client: Requesting a proof from the server...");
            sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: columns_to_verify.clone() })
                .await.expect("Failed to send message to server");


            let Some(Ok(transmission)) = stream.next().await else {
                tracing::error!("Failed to receive message from server");
                return Err(Box::from("Failed to receive message from server"));
            };
            tracing::trace!("Client received: {:?}", transmission);

            match transmission {
                ServerMessages::Columns { columns: received_columns } => {
                    tracing::debug!("client: received columns from server");
                    // let locally_derived_leaves_test = get_processed_column_leaves_from_file(&file_metadata, columns_to_verify.clone()).await;

                    tracing::debug!("verifying commitment");
                    let verification_result = client_verify_commitment(
                        &file_metadata.root,
                        &leaves_to_verify,
                        &columns_to_verify,
                        &received_columns,
                        get_PoS_soudness_n_cols(&file_metadata));

                    if verification_result.is_err() {
                        tracing::error!("Failed to verify colums");
                        //todo return error type
                        return Err(Box::from("failed_to_verify_columns".to_string()));
                    }

                    tracing::debug!("client: file verification successful!");
                    tracing::debug!("client: writing file to disk");

                    let mut file_handle = File::create(&file_metadata.filename).await?;
                    file_handle.write_all(&file_data).await?;

                    Ok(())
                }
                _ => {
                    tracing::error!("Unexpected server response");
                    Err(Box::from("Unknown server response"))
                }
            }
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

/// this is a thin wrapper for verify_compact_commit function
pub async fn request_proof(
    file_metadata: ClientOwnedFileMetadata,
    server_ip: String,
    security_bits: u8,
) -> Result<(), Box<dyn std::error::Error>> {
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
    file_metadata: &ClientOwnedFileMetadata,
    stream: &mut SerStream<ServerMessages>,
    sink: &mut DeSink<ClientMessages>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!("sending proof request for {} to server", &file_metadata.filename);

    // pick the random columns
    let cols_to_verify = get_columns_from_random_seed(
        1337, //todo refactor s.t. this is a function arg and not hardcoded
        get_PoS_soudness_n_cols(file_metadata),
        file_metadata.num_encoded_columns,
    );

    tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

    sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
        .await.expect("Failed to send message to server");


    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::debug!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::Columns { columns: received_columns } => {
            let locally_derived_leaves = get_processed_column_leaves_from_file(file_metadata, cols_to_verify.clone()).await;
            let verification_result = client_verify_commitment(
                &file_metadata.root,
                &locally_derived_leaves,
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
    let (root, commit) = server::convert_file_metadata_to_commit(&file_metadata)
        .map_err(|e| {
            tracing::error!("failed to convert file to commit: {:?}", e);
        })
        .expect("failed to convert file to commit");


    // let mut leaves = Vec::with_capacity(cols_to_verify.len());
    // for col in cols_to_verify {
    //     leaves.push(commit.hashes[col]);
    // }
    // leaves

    let extracted_columns = server_retreive_columns(&commit, &cols_to_verify);

    let extracted_leaves: Vec<Output<Blake3>> = extracted_columns
        .iter()
        .map(hash_column_to_digest::<Blake3>)
        .collect();

    extracted_leaves
}


pub fn convert_read_file_to_commit_only_leaves<D>(
    file_data: &Vec<u8>,
    columns_to_extract: &Vec<usize>,
) -> Result<(Vec<Output<D>>), Box<dyn std::error::Error>>
where
    D: Digest,
{
    let field_vector_file = fields::convert_byte_vec_to_field_elements_vec(file_data);
    //todo: ought to make customizable sizes for this
    let data_min_width = (field_vector_file.len() as f32).sqrt().ceil() as usize;
    let data_realized_width = data_min_width.next_power_of_two();
    let matrix_colums = (data_realized_width + 1).next_power_of_two();
    let matrix_rows = usize::div_ceil(field_vector_file.len(), matrix_colums);
    //field_vector_file.len() / matrix_colums + 1;

    let (data_realized_width_test, matrix_columns_test, soundness_test) = get_aspect_ratio_default_from_field_len(field_vector_file.len());
    let matrix_rows = usize::div_ceil(field_vector_file.len(), data_realized_width_test);

    let encoding = PoSEncoding::new_from_dims(data_realized_width, matrix_colums);


    // matrix (encoded as a vector)
    // XXX(zk) pad coeffs
    let mut coeffs_with_padding = vec![WriteableFt63::zero(); matrix_rows * data_realized_width];
    let mut encoded_coefs = vec![WriteableFt63::zero(); matrix_rows * matrix_colums];

    // local copy of coeffs with padding
    coeffs_with_padding
        .par_chunks_mut(data_realized_width)
        .zip(field_vector_file.par_chunks(data_realized_width))
        .for_each(|(base_row, row_to_copy)|
        base_row[..row_to_copy.len()].copy_from_slice(row_to_copy)
        );

    // now compute FFTs
    encoded_coefs.par_chunks_mut(matrix_colums)
        .zip(coeffs_with_padding.par_chunks(data_realized_width))
        .try_for_each(|(r, c)| {
            r[..c.len()].copy_from_slice(c);
            encoding.encode(r)
        })?;

    // now set up the hashing digests
    let mut digests = Vec::with_capacity(columns_to_extract.len());
    for _ in 0..columns_to_extract.len() {
        let mut new_hasher = D::new();
        new_hasher.update(<Output<D> as Default>::default());
        digests.push(new_hasher);
    }

    // for each row, update the digests from the requested columns
    for row in 0..matrix_rows {
        for (column_index, mut hasher) in columns_to_extract.iter()
            .zip(digests.iter_mut()) {
            encoded_coefs[row * matrix_colums + column_index].digest_update(hasher);
        }
    }
    let column_digest_results = digests
        .into_iter()
        .map(|hasher|
        hasher.finalize())
        .collect();

    // let mut column_digest_results: Vec<Output<D>> = Vec::with_capacity(digests.len());
    // for hasher in digests {
    //     column_digest_results.push(hasher.finalize());
    // }

    Ok(column_digest_results)
}

pub async fn client_request_and_verify_polynomial<D>(
    file_metadata: &ClientOwnedFileMetadata,
    server_ip: String,
) -> Result<(), Box<dyn std::error::Error>>
where
    D: Digest,
{
    tracing::info!("requesting polynomial evaluation from server");

    tracing::debug!("connecting to server {}", &server_ip);
    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (mut stream, mut sink) = wrap_stream::<ClientMessages, ServerMessages>(stream);

    let rng = &mut ChaCha8Rng::seed_from_u64(1337);
    let evaluation_point = WriteableFt63::random(rng);

    sink.send(ClientMessages::RequestPolynomialEvaluation { file_metadata: file_metadata.clone(), evaluation_point })
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::debug!("Client received: {:?}", transmission);

    match transmission {
        ServerMessages::PolynomialEvaluation { evaluation_result } => {
            tracing::debug!("client: received polynomial evaluation from server");

            let cols_to_verify = get_columns_from_random_seed(
                1337,
                get_PoS_soudness_n_cols(file_metadata),
                file_metadata.num_encoded_columns,
            );

            tracing::info!("client: Requesting a proof from the server...");
            tracing::trace!("client: requesting the following columns from the server: {:?}", cols_to_verify);

            sink.send(ClientMessages::RequestProof { file_metadata: file_metadata.clone(), columns_to_verify: cols_to_verify.clone() })
                .await.expect("Failed to send message to server");


            let Some(Ok(transmission)) = stream.next().await else {
                tracing::error!("Failed to receive message from server");
                return Err(Box::from("Failed to receive message from server"));
            };
            tracing::debug!("Client received: {:?}", transmission);

            match transmission {
                ServerMessages::Columns { columns: received_columns } => {
                    let locally_derived_leaves = get_processed_column_leaves_from_file(file_metadata, cols_to_verify.clone()).await;
                    let verification_result = client_verify_commitment(
                        &file_metadata.root,
                        &locally_derived_leaves,
                        &cols_to_verify,
                        &received_columns,
                        get_PoS_soudness_n_cols(file_metadata));

                    if verification_result.is_err() {
                        tracing::error!("Failed to verify colums");
                        //todo return error type
                        return Err(Box::from("failed_to_verify_columns".to_string()));
                    }

                    let local_evaluation_check =
                        lcpc_online::verifiable_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
                            &evaluation_point,
                            &evaluation_result,
                            file_metadata.num_rows,
                            file_metadata.num_columns,
                            &cols_to_verify,
                            &received_columns,
                        );

                    if local_evaluation_check.is_err() {
                        tracing::error!("Failed to verify polynomial evaluation");
                        Err(Box::from("failed_to_verify_polynomial_evaluation".to_string()))
                    } else {
                        Ok(())
                    }
                }

                ServerMessages::BadResponse { error } => {
                    tracing::error!("Polynomial evaluation failed: {}", error);
                    Err(Box::from(error))
                }
                _ => {
                    tracing::error!("Unexpected server response");
                    Err(Box::from("Unknown server response"))
                }
            }
        }
        ServerMessages::BadResponse { error } => {
            tracing::error!("Polynomial evaluation failed: {}", error);
            Err(Box::from(error))
        }
        _ => {
            tracing::error!("Unexpected server response");
            Err(Box::from("Unknown server response"))
        }
    }

    // todo!()
}

pub async fn reshape_file<D>(
    file_metadata: &ClientOwnedFileMetadata,
    server_ip: String,
    security_bits: u8,
    new_pre_encoded_columns: usize,
    new_encoded_columns: usize,
) -> Result<(), Box<dyn std::error::Error>>
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
        return Err(Box::from("Failed to receive message from server"));
    };
    tracing::debug!("Client received: {:?}", transmission);

    let ServerMessages::CompactCommit {
        root: new_root,
        file_metadata: new_file_metadata
    } = transmission else {
        return match transmission {
            ServerMessages::BadResponse { error } => {
                tracing::error!("File reshape failed: {}", error);
                Err(Box::from(error))
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                Err(Box::from("Unknown server response"))
            }
        }
    };

    let mut random_seed = ChaCha8Rng::seed_from_u64(1337);
    let evaluation_point = WriteableFt63::random(&mut random_seed);

    let requested_original_columns = get_columns_from_random_seed(
        1337,
        get_PoS_soudness_n_cols(file_metadata),
        file_metadata.num_encoded_columns,
    );
    let requested_new_columns = get_columns_from_random_seed(
        1337,
        get_PoS_soudness_n_cols(&new_file_metadata),
        new_file_metadata.num_encoded_columns,
    );

    sink.send(ClientMessages::RequestReshapeEvaluation {
        evaluation_point,
        columns_to_expand_original: requested_original_columns.clone(),
        columns_to_expand_new: requested_new_columns.clone(),
    }).await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return Err(Box::from("Failed to receive message from server"));
    };

    let ServerMessages::ReshapeEvaluation {
        evaluation_result,
        original_result_vector,
        original_columns,
        new_result_vector,
        new_columns,
    } = transmission else {
        return match transmission {
            ServerMessages::BadResponse { error } => {
                tracing::error!("File reshape failed: {}", error);
                Err(Box::from(error))
            }
            _ => {
                tracing::error!("Unknown server response: {:?}", transmission);
                Err(Box::from("Unknown server response"))
            }
        }
    };

    let old_results = lcpc_online::verifiable_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &original_result_vector,
        file_metadata.num_rows,
        file_metadata.num_columns,
        &requested_original_columns,
        &original_columns,
    );

    let new_results = lcpc_online::verifiable_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
        &evaluation_point,
        &new_result_vector,
        new_file_metadata.num_rows,
        new_file_metadata.num_columns,
        &requested_new_columns,
        &new_columns,
    );

    if old_results.is_err() || new_results.is_err() {
        tracing::error!("Failed to verify polynomial evaluation");

        sink.send(ClientMessages::ReshapeRejected).await.expect("Failed to send message to server");

        return Err(Box::from("failed_to_verify_polynomial_evaluation".to_string()));
    }

    sink.send(ClientMessages::ReshapeApproved).await.expect("Failed to send message to server");

    Ok(())
}