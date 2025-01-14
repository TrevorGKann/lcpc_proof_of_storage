use std::cmp::min;
use std::env;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use blake3::traits::digest;
use blake3::traits::digest::Output;
use blake3::{Hash, Hasher as Blake3};
use ff::{Field, PrimeField};
use futures::{SinkExt, StreamExt, TryFutureExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::RocksDb;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{formats::Json, Deserializer, Serializer};
use tracing::Instrument;
use ulid::Ulid;

use lcpc_2d::{LcCommit, LcEncoding, LcRoot};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::databases::{constants, FileMetadata, ServerHost, User};
use crate::fields::WriteableFt63;
use crate::fields::{
    convert_byte_vec_to_field_elements_vec, evaluate_field_polynomial_at_point, is_power_of_two,
    read_file_to_field_elements_vec,
};
use crate::lcpc_online::{
    convert_file_data_to_commit, form_side_vectors_for_polynomial_evaluation_from_point,
    get_PoS_soudness_n_cols, server_retreive_columns, verifiable_polynomial_evaluation,
    CommitDimensions, CommitOrLeavesOutput, CommitRequestType,
};
use crate::networking::shared;
use crate::networking::shared::wrap_stream;
use crate::networking::shared::ClientMessages;
use crate::networking::shared::ServerMessages;
use crate::networking::shared::ServerMessages::ErrorResponse;
use crate::{fields, PoSColumn, PoSCommit};

type InternalServerMessage = ServerMessages;

// static DB: Lazy<Surreal<Db>> = Lazy::new(Surreal::init);

#[tracing::instrument]
pub async fn server_main(port: u16, verbosity: u8) -> Result<()> {
    tracing::info!("Server starting, verbosity level: {:?}", verbosity);

    // failed attempt at lazy init
    // tracing::debug!("Connecting to local database");
    // DB.connect::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    // DB.use_ns(constants::SERVER_NAMESPACE).use_db(constants::SERVER_DATABASE_NAME).await?;

    tracing::debug!("Binding to port {:?}", port);
    let listening_address = format!("0.0.0.0:{}", port);

    let listener = TcpListener::bind(listening_address)
        .await
        .map_err(|e| tracing::error!("server failed to start: {:?}", e))
        .expect("failed to initialize listener");

    tracing::info!(
        "Server started on port {:?}",
        listener.local_addr().unwrap()
    );

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(async move { handle_client_loop(stream).await });
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
pub(crate) async fn handle_client_loop(mut stream: TcpStream) {
    let (mut stream, mut sink) = wrap_stream::<InternalServerMessage, ClientMessages>(stream);

    while let Some(Ok(transmission)) = stream.next().await {
        // tracing::info!("Server received: {:?}", transmission);
        let request_result = match transmission {
            ClientMessages::NewUser { username, password } => {
                handle_client_new_user(username, password).await
            }
            ClientMessages::UserLogin { username, password } => {
                handle_client_user_login(username, password).await
            }
            ClientMessages::UploadNewFile {
                filename,
                file,
                columns,
                encoded_columns,
            } => handle_client_upload_new_file(filename, file, columns, encoded_columns).await,
            ClientMessages::RequestFile { file_metadata } => {
                handle_client_request_file(file_metadata).await
            }
            ClientMessages::StartUploadNewFileByChunks {
                filename,
                columns,
                encoded_columns,
            } => {
                handle_client_start_upload_file_by_chunks(filename, columns, encoded_columns).await
            }
            ClientMessages::UploadFileChunk {
                file_ulid,
                chunk,
                last_chunk,
            } => handle_client_upload_file_chunk(file_ulid, chunk, last_chunk).await,
            ClientMessages::RequestFileRow { file_metadata, row } => {
                handle_client_request_file_row(file_metadata, row).await
            }
            ClientMessages::EditFileBytes {
                file_metadata,
                start_byte,
                replacement_bytes: file,
            } => handle_client_edit_file_bytes(file_metadata, start_byte, file).await,
            ClientMessages::RequestEditEvaluation {
                old_file_metadata,
                new_file_metadata,
                evaluation_point,
                columns_to_expand,
                requested_unencoded_row_range_inclusive,
            } => {
                handle_client_append_or_edit_eval_request(
                    &old_file_metadata,
                    &new_file_metadata,
                    &evaluation_point,
                    &columns_to_expand,
                    Some(requested_unencoded_row_range_inclusive),
                )
                .await
            }
            ClientMessages::AppendToFile {
                file_metadata,
                append_data,
            } => handle_client_append_to_file(file_metadata, append_data).await,
            ClientMessages::RequestAppendEvaluation {
                old_file_metadata,
                new_file_metadata,
                evaluation_point,
                columns_to_expand,
            } => {
                handle_client_append_or_edit_eval_request(
                    &old_file_metadata,
                    &new_file_metadata,
                    &evaluation_point,
                    &columns_to_expand,
                    None,
                )
                .await
            }
            shared::ClientMessages::EditOrAppendResponse {
                new_file_metadata,
                old_file_metadata,
                accepted,
            } => {
                handle_client_append_or_edit_response(
                    &old_file_metadata,
                    &new_file_metadata,
                    accepted,
                )
                .await
            }
            ClientMessages::RequestEncodedColumn { file_metadata, row } => {
                handle_client_request_encoded_column(file_metadata, row).await
            }
            ClientMessages::RequestProof {
                file_metadata,
                columns_to_verify,
            } => handle_client_request_proof(file_metadata, columns_to_verify).await,
            ClientMessages::RequestPolynomialEvaluation {
                file_metadata,
                evaluation_point,
            } => {
                handle_client_request_polynomial_evaluation(file_metadata, &evaluation_point).await
            }
            ClientMessages::RequestFileReshape {
                file_metadata,
                new_pre_encoded_columns,
                new_encoded_columns,
            } => {
                handle_client_request_file_reshape(
                    file_metadata,
                    new_pre_encoded_columns,
                    new_encoded_columns,
                )
                .await
            }
            ClientMessages::RequestReshapeEvaluation {
                old_file_metadata,
                new_file_metadata,
                evaluation_point,
                columns_to_expand_original,
                columns_to_expand_new,
            } => {
                handle_client_request_reshape_evaluation(
                    &old_file_metadata,
                    &new_file_metadata,
                    &evaluation_point,
                    &columns_to_expand_original,
                    &columns_to_expand_new,
                )
                .await
            }
            ClientMessages::ReshapeResponse {
                new_file_metadata,
                old_file_metadata,
                accepted,
            } => {
                handle_client_reshape_response(&old_file_metadata, &new_file_metadata, accepted)
                    .await
            }
            ClientMessages::DeleteFile { file_metadata } => {
                handle_client_delete_file(file_metadata).await
            }
            ClientMessages::ClientKeepAlive => Ok(ServerMessages::ServerKeepAlive),
        };

        let response = request_result.unwrap_or_else(|error| {
            make_bad_response(format!("Server failed to fulfil operation: {:?}", error))
        });

        //todo: remove this and implement another way to send the responses of the internal
        // functions while still possibly being able to send a keep alive message from within
        // the functions depending on how long each interaction takes
        sink.send(response)
            .await
            .expect("Failed to send message to client");
    }
}

#[tracing::instrument]
async fn handle_client_new_user(
    username: String,
    password: String,
) -> Result<InternalServerMessage> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(password.as_bytes());
    let hashed_password = hasher.finalize();

    let new_user = User {
        id_string: username.clone(),
        hashed_password: hashed_password.to_string(),
    };

    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;

    // check if the username is already taken
    let None: Option<User> = db
        .select((constants::SERVER_USER_TABLE, username.clone()))
        .await?
    else {
        bail!("Username already taken");
    };

    // insert the new user into the database
    let created: Option<User> = db.create(("user", username)).content(new_user).await?;
    Ok(ServerMessages::UserLoginResponse { success: true })
}

#[tracing::instrument]
async fn handle_client_user_login(
    username: String,
    password: String,
) -> Result<InternalServerMessage> {
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(password.as_bytes());
    let hashed_password = hasher.finalize();

    let Some(user): Option<User> = db.select((constants::SERVER_USER_TABLE, username)).await?
    else {
        bail!("user not found")
    };
    ensure!(
        user.hashed_password == hashed_password.to_string(),
        "Incorrect password"
    );

    Ok(ServerMessages::UserLoginResponse { success: true })
    // todo: logic here to handle the user login for the rest of this thread
}

/// save the file to the server
#[tracing::instrument(skip_all)]
async fn handle_client_upload_new_file(
    filename: String,
    file_data: Vec<u8>,
    pre_encoded_columns: usize,
    encoded_columns: usize,
) -> Result<InternalServerMessage> {
    // check if rows and columns are valid first
    ensure!(
        lcpc_online::dims_ok(pre_encoded_columns, encoded_columns),
        "Invalid rows or columns"
    );

    // parse the filename to remove all leading slashes
    use crate::databases::FileMetadata;
    use crate::lcpc_online;
    use std::path::Path;

    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::Commit(commit) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: pre_encoded_columns,
            num_encoded_columns: encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let new_upload_id = Ulid::new();
    let local_file_location = get_file_location_from_id(&new_upload_id);
    tracing::debug!("Writing new file to {}", local_file_location.display());
    tokio::fs::write(&local_file_location, &file_data)
        .await
        .context("failed while writing file from client")?;

    tracing::info!("server: appending file metadata to database");
    let file_metadata = FileMetadata {
        id_ulid: new_upload_id,
        filename: filename,
        // filename: Path::new(&filename).file_name().unwrap().to_str().unwrap().to_string(),
        num_rows: commit.n_rows,
        num_columns: pre_encoded_columns,
        num_encoded_columns: encoded_columns,
        filesize_in_bytes: file_data.len(),
        stored_server: ServerHost {
            server_name: None,
            server_ip: "".to_string(),
            server_port: 0,
        },
        root: commit.get_root(),
    };

    {
        // database lock scope
        let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
        db.use_ns(constants::SERVER_NAMESPACE)
            .use_db(constants::SERVER_DATABASE_NAME)
            .await?;
        // db.create::<Vec<FileMetadata>>(constants::SERVER_METADATA_TABLE)
        //     .content(&file_metadata).await?;
        db.create::<Option<FileMetadata>>((
            constants::SERVER_METADATA_TABLE,
            file_metadata.id_ulid.to_string(),
        ))
        .content(&file_metadata)
        .await?;
    }

    Ok(ServerMessages::CompactCommit { file_metadata })
}

#[tracing::instrument(skip_all)]
async fn handle_client_start_upload_file_by_chunks(
    filename: String,
    pre_encoded_columns: usize,
    encoded_columns: usize,
) -> Result<InternalServerMessage> {
    todo!()
}

#[tracing::instrument(skip(chunk))]
async fn handle_client_upload_file_chunk(
    file_ulid: Ulid,
    chunk: Vec<u8>,
    last_chunk: bool,
) -> Result<InternalServerMessage> {
    todo!()
}

#[tracing::instrument(skip_all)]
async fn handle_client_request_file(file_metadata: FileMetadata) -> Result<InternalServerMessage> {
    // get the requested file from the server
    // send the file to the client

    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);

    let file = tokio::fs::read(&file_handle).await?;

    Ok(ServerMessages::File { file })
}

#[tracing::instrument]
async fn handle_client_request_file_row(
    file_metadata: FileMetadata,
    row: usize,
) -> Result<InternalServerMessage> {
    // get the requested row from the file
    // send the row to the client

    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);
    let mut file = tokio::fs::File::open(&file_handle).await?;

    let seek_pointer = file_metadata.num_columns * row;
    file.seek(SeekFrom::Start(seek_pointer as u64));

    let mut file_data = Vec::<u8>::with_capacity(file_metadata.num_columns);
    file.take(file_metadata.num_columns as u64)
        .read_to_end(&mut file_data);

    Ok(ServerMessages::FileRow { row: file_data })
}

#[tracing::instrument(skip_all)]
async fn handle_client_edit_file_bytes(
    file_metadata: FileMetadata,
    start_byte: usize,
    new_file_data: Vec<u8>,
) -> Result<InternalServerMessage> {
    // edit the requested row in the file

    check_file_metadata(&file_metadata)?;

    // ensure!(new_file_data.len() == file_metadata.num_columns, "new file data is not the correct length".to_string());

    let old_file_handle = get_file_handle_from_metadata(&file_metadata);
    let new_id = Ulid::new();
    let new_file_handle = get_file_location_from_id(&new_id);

    //scope for file editing
    {
        tokio::fs::copy(&old_file_handle, &new_file_handle).await?;
        // let mut new_file = tokio::fs::File::open(&new_file_handle).await?;
        let mut new_file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&new_file_handle)
            .await?;

        new_file.seek(SeekFrom::Start(start_byte as u64)).await?;
        new_file.write_all(&new_file_data).await?;
        new_file.flush().await?;
    }
    // optimization: stream the file in only once and live update the edit, since I have to convert it anyways

    let new_file_data = tokio::fs::read(&new_file_handle).await?;

    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&new_file_data);
    let CommitOrLeavesOutput::Commit(updated_commit) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: file_metadata.num_columns,
            num_encoded_columns: file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let updated_file_metadata = FileMetadata {
        id_ulid: new_id,
        root: updated_commit.get_root(),
        filesize_in_bytes: new_file_data.len(),
        ..file_metadata
    };

    // update the database to include the new entry;
    // one of the entries will be deleted after the client responds
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;
    db.create::<Option<FileMetadata>>((
        constants::SERVER_METADATA_TABLE,
        updated_file_metadata.id_ulid.to_string(),
    ))
    .content(&updated_file_metadata)
    .await?;

    Ok(ServerMessages::CompactCommit {
        file_metadata: updated_file_metadata,
    })
}

#[tracing::instrument(skip_all)]
async fn handle_client_append_to_file(
    file_metadata: FileMetadata,
    mut file_data_to_append: Vec<u8>,
) -> Result<InternalServerMessage> {
    // append the file to the requested file

    check_file_metadata(&file_metadata)?;

    let old_file_handle = get_file_handle_from_metadata(&file_metadata);
    let new_id = Ulid::new();
    let new_file_handle = get_file_location_from_id(&new_id);

    tracing::debug!(
        "server: appending new data to file: {:?} bytes -> {:?}",
        file_data_to_append.len(),
        old_file_handle
    );
    // tokio::fs::copy(&old_file_handle, &new_file_handle).await?;

    let mut new_file_data = tokio::fs::read(&old_file_handle).await?;
    new_file_data.append(&mut file_data_to_append);

    tokio::fs::write(&new_file_handle, &new_file_data)
        .await
        .context("failed while writing file from client to new file location")?;
    tracing::debug!(
        "server: appended file now exists at {:?} with length {}",
        new_file_handle,
        new_file_data.len()
    );

    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&new_file_data);
    let CommitOrLeavesOutput::Commit(updated_commit) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: file_metadata.num_columns,
            num_encoded_columns: file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let updated_file_metadata = FileMetadata {
        id_ulid: new_id,
        root: updated_commit.get_root(),
        num_rows: updated_commit.n_rows,
        filesize_in_bytes: new_file_data.len(),
        ..file_metadata
    };

    // update the database to include the new entry;
    // one of the entries will be deleted after the client responds
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;
    db.create::<Option<FileMetadata>>((
        constants::SERVER_METADATA_TABLE,
        updated_file_metadata.id_ulid.to_string(),
    ))
    .content(&updated_file_metadata)
    .await?;

    Ok(ServerMessages::CompactCommit {
        file_metadata: updated_file_metadata,
    })
}

#[tracing::instrument]
async fn handle_client_request_encoded_column(
    file_metadata: FileMetadata,
    row: usize,
) -> Result<InternalServerMessage> {
    // get the requested column from the file
    // send the column to the client

    check_file_metadata(&file_metadata)?;

    unimplemented!("handle_client_request_encoded_column");
}

#[tracing::instrument]
async fn handle_client_request_proof(
    file_metadata: FileMetadata,
    requested_columns: Vec<usize>,
) -> Result<InternalServerMessage> {
    // get the requested proof from the file
    // send the proof to the client

    tracing::trace!(
        "server: requested columns for client proof: {:?}",
        requested_columns
    );
    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);
    tracing::trace!(
        "reading file for proof at {}",
        file_handle.to_str().unwrap()
    );
    let file_data = tokio::fs::read(&file_handle).await?;
    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::ColumnsWithPath(column_collection) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::ColumnsWithPath(requested_columns),
        CommitDimensions::Specified {
            num_pre_encoded_columns: file_metadata.num_columns,
            num_encoded_columns: file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to columns")
    };

    // let column_collection = server_retreive_columns(&commit, &requested_columns);

    for column in column_collection.iter().take(5) {
        tracing::trace!("server: sending leaf to client: {:x}", column.path[0]);
    }

    Ok(ServerMessages::Columns {
        columns: column_collection,
    })
}

#[tracing::instrument]
async fn handle_client_request_polynomial_evaluation(
    file_metadata: FileMetadata,
    evaluation_point: &WriteableFt63,
) -> Result<InternalServerMessage> {
    // get the requested polynomial evaluation from the file
    // send the evaluation to the client
    tracing::info!(
        "server: client requested polynomial evaluation of {:?} at {:?}",
        &file_metadata.filename,
        evaluation_point
    );
    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);
    let file_data = tokio::fs::read(&file_handle).await?;
    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::Commit(commit) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: file_metadata.num_columns,
            num_encoded_columns: file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let (evaluation_left_vector, _) = form_side_vectors_for_polynomial_evaluation_from_point(
        evaluation_point,
        commit.n_rows,
        commit.n_cols,
    );

    let result_vector = verifiable_polynomial_evaluation(&commit, &evaluation_left_vector);

    Ok(ServerMessages::PolynomialEvaluation {
        evaluation_result: result_vector,
    })

    // unimplemented!("handle_client_request_polynomial_evaluation");
}

#[tracing::instrument]
async fn handle_client_delete_file(file_metadata: FileMetadata) -> Result<ServerMessages> {
    tracing::info!(
        "server: client requested to delete file: {}",
        file_metadata.filename
    );
    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);
    let db_delete_result: Option<FileMetadata> = {
        //database scope
        tracing::debug!(
            "deleting file from database: {}; with ID: {}",
            file_metadata.filename,
            file_metadata.id_ulid.to_string()
        );
        let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
        db.use_ns(constants::SERVER_NAMESPACE)
            .use_db(constants::SERVER_DATABASE_NAME)
            .await?;
        db.delete::<Option<FileMetadata>>((
            constants::SERVER_METADATA_TABLE,
            file_metadata.id_ulid.to_string(),
        ))
        .await?
    };

    tracing::debug!("deleting file from disk: {}", file_handle.to_str().unwrap());
    let os_delete_result = tokio::fs::remove_file(file_handle).await;

    let mut return_string = "".to_string();

    if os_delete_result.is_err() {
        return_string
            .push_str(format!("error deleting file: {}; ", os_delete_result.unwrap_err()).as_str());
    }
    if db_delete_result.is_none() {
        if return_string.len() != 0 {
            return_string.push_str("; ")
        }
        return_string.push_str(
            "error deleting file metadata from local database"
                .to_string()
                .as_str(),
        );
    }

    ensure!(return_string.len() == 0, return_string);

    Ok(ServerMessages::FileDeleted {
        filename: file_metadata.filename,
    })
}

#[tracing::instrument]
async fn handle_client_request_file_reshape(
    file_metadata: FileMetadata,
    new_pre_encoded_columns: usize,
    new_encoded_columns: usize,
) -> Result<ServerMessages> {
    check_file_metadata(&file_metadata)?;

    let file_handle = get_file_handle_from_metadata(&file_metadata);
    let file_data = tokio::fs::read(&file_handle).await?;
    let encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::Commit(updated_commit) = convert_file_data_to_commit(
        &encoded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: new_pre_encoded_columns,
            num_encoded_columns: new_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    // I ensure on the reshape accepted to change the old filename to the new ID when I delete the
    // old record in the database
    let updated_file_metadata = FileMetadata {
        id_ulid: Ulid::new(),
        num_rows: updated_commit.n_rows,
        num_columns: new_pre_encoded_columns,
        num_encoded_columns: new_encoded_columns,
        root: updated_commit.get_root(),
        ..file_metadata
    };

    Ok(ServerMessages::CompactCommit {
        file_metadata: updated_file_metadata,
    })
}

#[tracing::instrument]
async fn handle_client_request_reshape_evaluation(
    old_file_metadata: &FileMetadata,
    new_file_metadata: &FileMetadata,
    evaluation_point: &WriteableFt63,
    columns_to_expand_original: &Vec<usize>,
    columns_to_expand_new: &Vec<usize>,
) -> Result<ServerMessages> {
    check_file_metadata(&old_file_metadata)?;
    check_file_metadata(&new_file_metadata)?;

    // Both files have the same stored data at the id of the old metadata, we only need to read it once
    let old_file_handle = get_file_handle_from_metadata(&old_file_metadata);
    let file_data = tokio::fs::read(&old_file_handle).await?;
    let fielded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);

    let CommitOrLeavesOutput::Commit(old_commit) = convert_file_data_to_commit(
        &fielded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: old_file_metadata.num_columns,
            num_encoded_columns: old_file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let CommitOrLeavesOutput::Commit(new_commit) = convert_file_data_to_commit(
        &fielded_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: new_file_metadata.num_columns,
            num_encoded_columns: new_file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    // let (_, old_commit) = convert_file_metadata_to_commit(&old_file_metadata)?;
    // let (_, new_commit) = convert_file_metadata_to_commit(&new_file_metadata)?;

    let (evaluation_left_vector_for_old_commit, _) =
        form_side_vectors_for_polynomial_evaluation_from_point(
            evaluation_point,
            old_commit.n_rows,
            old_commit.n_per_row,
        );
    let result_vector_for_old_commit =
        verifiable_polynomial_evaluation(&old_commit, &evaluation_left_vector_for_old_commit);
    let columns_for_old_commit = server_retreive_columns(&old_commit, &columns_to_expand_original);

    let (evaluation_left_vector_for_new_commit, _) =
        form_side_vectors_for_polynomial_evaluation_from_point(
            evaluation_point,
            new_commit.n_rows,
            new_commit.n_per_row,
        );
    let result_vector_for_new_commit =
        verifiable_polynomial_evaluation(&new_commit, &evaluation_left_vector_for_new_commit);
    let columns_for_new_commit = server_retreive_columns(&new_commit, &columns_to_expand_new);

    let expected_eval =
        fields::evaluate_field_polynomial_at_point(&fielded_file_data, &evaluation_point);

    Ok(ServerMessages::ReshapeEvaluation {
        expected_result: expected_eval,
        original_result_vector: result_vector_for_old_commit,
        original_columns: columns_for_old_commit,
        new_result_vector: result_vector_for_new_commit,
        new_columns: columns_for_new_commit,
    })
}

#[tracing::instrument]
async fn handle_client_reshape_response(
    old_file_metadata: &FileMetadata,
    new_file_metadata: &FileMetadata,
    accepted: bool,
) -> Result<ServerMessages> {
    tracing::info!(
        "Client responded to a reshape request: {:?} to {:?} was accepted? {}",
        &old_file_metadata,
        &new_file_metadata,
        accepted
    );
    check_file_metadata(&old_file_metadata)?;
    check_file_metadata(&new_file_metadata)?;

    let old_file_handle = get_file_handle_from_metadata(&old_file_metadata);
    let new_file_handle = get_file_handle_from_metadata(&new_file_metadata);

    // we now have two database entries for the same file, the new one has a bad ID, so we'll have to
    // change the filename and delete the old entry if it's accepted and only have to delete the new
    // entry if it's rejected.
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;

    let resulting_file_metadata: &FileMetadata = if accepted {
        tokio::fs::rename(&old_file_handle, &new_file_handle).await?;
        let db_delete_result: Option<FileMetadata> = db
            .delete((
                constants::SERVER_METADATA_TABLE,
                &old_file_metadata.id_ulid.to_string(),
            ))
            .await?;

        new_file_metadata
    } else {
        let db_delete_result: Option<FileMetadata> = db
            .delete((
                constants::SERVER_METADATA_TABLE,
                &old_file_metadata.id_ulid.to_string(),
            ))
            .await?;

        old_file_metadata
    };

    Ok(ServerMessages::CompactCommit {
        file_metadata: resulting_file_metadata.to_owned(),
    })
}

#[tracing::instrument]
async fn handle_client_append_or_edit_eval_request(
    old_file_metadata: &FileMetadata,
    new_file_metadata: &FileMetadata,
    evaluation_point: &WriteableFt63,
    columns_to_expand: &Vec<usize>,
    requested_unencoded_row_range_inclusive_for_edits: Option<(usize, usize)>,
) -> Result<ServerMessages> {
    check_file_metadata(&old_file_metadata)?;
    check_file_metadata(&new_file_metadata)?;

    let old_file_handle = get_file_handle_from_metadata(&old_file_metadata);
    let new_file_handle = get_file_handle_from_metadata(&new_file_metadata);

    tracing::debug!("reading old file from {:?}", &old_file_handle);
    let old_file_data = tokio::fs::read(&old_file_handle).await?;
    tracing::debug!("reading new file from {:?}", &new_file_handle);
    let new_file_data = tokio::fs::read(&new_file_handle).await?;

    let fielded_old_file_data = convert_byte_vec_to_field_elements_vec(&old_file_data);
    let fielded_new_file_data = convert_byte_vec_to_field_elements_vec(&new_file_data);

    let CommitOrLeavesOutput::Commit(old_commit) = convert_file_data_to_commit(
        &fielded_old_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: old_file_metadata.num_columns,
            num_encoded_columns: old_file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let CommitOrLeavesOutput::Commit(new_commit) = convert_file_data_to_commit(
        &fielded_new_file_data,
        CommitRequestType::Commit,
        CommitDimensions::Specified {
            num_pre_encoded_columns: new_file_metadata.num_columns,
            num_encoded_columns: new_file_metadata.num_encoded_columns,
        },
    )?
    else {
        bail!("Unexpected failure to convert file to commitment")
    };

    let (evaluation_left_vector_for_old_commit, _) =
        form_side_vectors_for_polynomial_evaluation_from_point(
            evaluation_point,
            old_commit.n_rows,
            old_commit.n_per_row,
        );
    let result_vector_for_old_commit =
        verifiable_polynomial_evaluation(&old_commit, &evaluation_left_vector_for_old_commit);
    let columns_for_old_commit = server_retreive_columns(&old_commit, columns_to_expand);

    let (evaluation_left_vector_for_new_commit, _) =
        form_side_vectors_for_polynomial_evaluation_from_point(
            evaluation_point,
            new_commit.n_rows,
            new_commit.n_per_row,
        );
    let result_vector_for_new_commit =
        verifiable_polynomial_evaluation(&new_commit, &evaluation_left_vector_for_new_commit);
    let columns_for_new_commit = server_retreive_columns(&new_commit, columns_to_expand);

    match requested_unencoded_row_range_inclusive_for_edits {
        None => {
            // the case for appends, the client only needs one line
            let start_of_edited_row =
                (old_file_metadata.num_rows - 1) * old_file_metadata.num_columns;
            let end_of_edited_row = if (old_file_metadata.num_rows < new_file_metadata.num_rows) {
                (old_file_metadata.num_rows * old_file_metadata.num_columns) - 1
            } else {
                new_file_metadata
                    .filesize_in_bytes
                    .div_ceil(WriteableFt63::CAPACITY as usize)
            }; // optimization: can probably just be a `min` instead of an if-then-else

            let edited_unencoded_row =
                fielded_new_file_data[start_of_edited_row..=end_of_edited_row].to_vec();

            Ok(ServerMessages::AppendEvaluation {
                original_result_vector: result_vector_for_old_commit,
                original_columns: columns_for_old_commit,
                new_result_vector: result_vector_for_new_commit,
                new_columns: columns_for_new_commit,
                edited_unencoded_row,
            })
        }
        Some((start, finish)) => {
            // the case for edits, the client needs multiple lines
            let start_of_edited_row_in_bytes =
                start * old_file_metadata.num_columns * WriteableFt63::BYTE_CAPACITY as usize;
            let end_of_original_row_in_bytes = ((finish + 1)
                * old_file_metadata.num_columns
                * WriteableFt63::BYTE_CAPACITY as usize)
                - 1;
            let end_of_original_row_in_bytes =
                min(end_of_original_row_in_bytes, old_file_data.len());

            let original_unencoded_rows =
                old_file_data[start_of_edited_row_in_bytes..=end_of_original_row_in_bytes].to_vec();

            Ok(ServerMessages::EditEvaluation {
                original_result_vector: result_vector_for_old_commit,
                original_columns: columns_for_old_commit,
                new_result_vector: result_vector_for_new_commit,
                new_columns: columns_for_new_commit,
                original_unencoded_rows,
            })
        }
    }
}

#[tracing::instrument]
async fn handle_client_append_or_edit_response(
    old_file_metadata: &FileMetadata,
    new_file_metadata: &FileMetadata,
    accepted: bool,
) -> Result<ServerMessages> {
    tracing::info!(
        "Client responded to an edit/append request: {:?} to {:?} was accepted? {}",
        &old_file_metadata,
        &new_file_metadata,
        accepted
    );
    check_file_metadata(&old_file_metadata)?;
    check_file_metadata(&new_file_metadata)?;

    let old_file_handle = get_file_handle_from_metadata(&old_file_metadata);
    let new_file_handle = get_file_handle_from_metadata(&new_file_metadata);

    // we now have two database entries for the same file, the new one has a bad ID, so we'll have to
    // change the filename and delete the old entry if it's accepted and only have to delete the new
    // entry if it's rejected.
    let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await?;
    db.use_ns(constants::SERVER_NAMESPACE)
        .use_db(constants::SERVER_DATABASE_NAME)
        .await?;

    // this is slightly different from the reshape request since we also have to delete the unaccepted file
    let resulting_file_metadata: &FileMetadata = if accepted {
        tokio::fs::rename(&old_file_handle, &new_file_handle).await?;
        let db_delete_result: Option<FileMetadata> = db
            .delete((
                constants::SERVER_METADATA_TABLE,
                &old_file_metadata.id_ulid.to_string(),
            ))
            .await?;

        tokio::fs::remove_file(&old_file_handle).await?;

        new_file_metadata
    } else {
        let db_delete_result: Option<FileMetadata> = db
            .delete((
                constants::SERVER_METADATA_TABLE,
                &old_file_metadata.id_ulid.to_string(),
            ))
            .await?;

        tokio::fs::remove_file(&new_file_handle).await?;

        old_file_metadata
    };

    Ok(ServerMessages::CompactCommit {
        file_metadata: resulting_file_metadata.to_owned(),
    })
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

    let soundness =
        get_soundness_from_matrix_dims(num_pre_encoded_columns, num_encoded_matrix_columns);

    (
        num_pre_encoded_columns,
        num_encoded_matrix_columns,
        soundness,
    )
}

pub(crate) fn get_soundness_from_matrix_dims(
    pre_encoded_cols: usize,
    encoded_cols: usize,
) -> usize {
    let soundness_denominator: f64 =
        ((1f64 + (pre_encoded_cols as f64 / encoded_cols as f64)) / 2f64).log2();
    let theoretical_min = (-128f64 / soundness_denominator).ceil() as usize;
    // can't test more than the number of total columns, sometimes theoretical min is above that.
    // Testing all columns is sufficient for perfect soundness
    min(theoretical_min, encoded_cols)
}

pub fn get_aspect_ratio_default_from_file_len<Field: PrimeField>(
    file_len: usize,
) -> (usize, usize, usize) {
    tracing::debug!("file_len: {}", file_len);
    let write_out_byte_width = (Field::CAPACITY / 8) as usize;
    let field_len = usize::div_ceil(file_len, write_out_byte_width);
    tracing::debug!("field_len: {}", field_len);

    ///num_pre_encoded_columns, num_encoded_matrix_columns, soundness
    get_aspect_ratio_default_from_field_len(field_len)
}

fn make_bad_response(message: String) -> InternalServerMessage {
    tracing::error!("{}", message);
    ServerMessages::ErrorResponse { error: message }
}

fn check_file_metadata(file_metadata: &FileMetadata) -> Result<()> {
    // todo:
    Ok(())
}

fn get_file_handle_from_metadata(file_metadata: &FileMetadata) -> PathBuf {
    // format!("PoS_server_files/{}", file_metadata.id)
    get_file_location_from_id(&file_metadata.id_ulid)
}

fn get_file_location_from_id(id: &Ulid) -> PathBuf {
    let mut path = env::current_dir().unwrap();
    path.push(constants::SERVER_FILE_FOLDER);

    //check that directory folder exists
    if !path.exists() {
        std::fs::create_dir(&path).unwrap();
    }

    path.push(format!("{}.{}", id.to_string(), constants::FILE_EXTENSION));
    path
}
