#[cfg(test)]
pub mod network_tests {
    use std::time::Duration;

    use anyhow::{bail, ensure, Result};
    use blake3::Hasher as Blake3;
    use blake3::traits::digest::Output;
    use ff::PrimeField;
    use serial_test::serial;
    use surrealdb::engine::local::RocksDb;
    use surrealdb::Surreal;
    // use pretty_assertions::assert_eq;
    use tokio::fs;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio::time::sleep;
    use ulid::Ulid;

    use crate::databases::*;
    use crate::fields::{convert_byte_vec_to_field_elements_vec, random_writeable_field_vec};
    use crate::fields::writable_ft63::WriteableFt63;
    use crate::lcpc_online::{CommitDimensions, CommitOrLeavesOutput, CommitRequestType, convert_file_data_to_commit, get_PoS_soudness_n_cols, hash_column_to_digest, server_retreive_columns};
    use crate::networking::client;
    use crate::networking::server::handle_client_loop;
    use crate::tests::tests::Cleanup;

    use super::*;

    async fn test_start_server(port: u16) {
        tracing::info!("Server starting");

        let listening_address = format!("0.0.0.0:{}", port);

        let listener = TcpListener::bind(listening_address).await
            .map_err(|e| tracing::error!("server failed to start: {:?}", e))
            .expect("failed to initialize listener");

        tracing::info!("Server started on port {:?}", listener.local_addr().unwrap());

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move { handle_client_loop(stream).await });
            }
        });
    }

    fn start_tracing_for_tests() -> Result<()> {
        let subscriber = tracing_subscriber::fmt()
            .pretty()
            .compact()
            .with_file(true)
            .with_line_number(true)
            .with_max_level(tracing::Level::DEBUG)
            .finish();

        // use that subscriber to process traces emitted after this point
        tracing::subscriber::set_global_default(subscriber)?;
        Ok(())
    }

    async fn start_test_with_server_on_random_port_and_get_port(test_name: String) -> u16 {
        start_tracing_for_tests();
        tracing::info!("Starting test {}", test_name);

        // select a random port
        let port = rand::random::<u16>();
        tokio::spawn(test_start_server(port)).await;
        port
    }

    #[tokio::test]
    #[serial]
    async fn upload_and_delete_file_test() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        // let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_file_test".to_string()).await;

        // upload the file
        let response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        ).await;

        tracing::debug!("client received: {:?}", response);

        let metadata = response.unwrap();

        assert_eq!(metadata.filename, source_file);
        assert_eq!(metadata.num_columns, 4); //todo uncomment once implemented
        assert_eq!(metadata.num_encoded_columns, 8);

        tracing::debug!("client requesting file deletion");
        client::delete_file(
            metadata,
            format!("localhost:{}", port),
        ).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn upload_then_verify() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_verify".to_string()).await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata = crate::networking::client::get_client_metadata_from_database_by_filename(
            &"test.txt".to_string(),
        ).await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(
                metadata,
                format!("localhost:{}", port),
            ).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        ).await;

        tracing::debug!("client received: {:?}", response);

        let metadata = response.unwrap();

        tracing::info!("requesting proof");
        let proof_response = client::request_proof(metadata, format!("localhost:{}", port), 0).await;
        tracing::debug!("client received: {:?}", proof_response);

        proof_response.unwrap();
    }

    // #[tokio::test]
    // #[serial]
    // async fn test_different_column_hashing_methods_agree() {
    //     start_tracing_for_tests();
    //     tracing::info!("starting test_different_column_hashing_methods_agree");
    //
    //     let test_file = "test_files/test.txt";
    //
    //     let (root, commit, file_metadata)
    //         = crate::networking::server::convert_file_to_commit_internal(test_file, None).unwrap();
    //
    //
    //     let cols_to_verify = crate::networking::client::get_columns_from_random_seed(
    //         1337,
    //         // get_PoS_soudness_n_cols(&file_metadata),
    //         2,
    //         file_metadata.num_encoded_columns);
    //
    //     let leaves_from_file = get_processed_column_leaves_from_file(&file_metadata, cols_to_verify.clone()).await;
    //
    //     let server_columns = server_retreive_columns(&commit, &cols_to_verify);
    //     let server_leaves: Vec<Output<Blake3>> = server_columns
    //         .iter()
    //         .map(hash_column_to_digest::<Blake3>)
    //         .collect();
    //
    //     let mut file_data = fs::read(test_file).await.unwrap();
    //     let mut encoded_file_data = convert_byte_vec_to_field_elements_vec(&file_data);
    //     let CommitOrLeavesOutput::Leaves(streamed_file_leaves)
    //         = convert_file_data_to_commit::<Blake3, _>(&encoded_file_data,
    //                                                    CommitRequestType::Leaves(cols_to_verify.clone()),
    //                                                    file_metadata.into()).unwrap()
    //     else {
    //         panic!("Non-leaf result from file conversion when leaves were expected")
    //     };
    //
    //     tracing::debug!("commit's hashes:");
    //     for i in 0..(commit.hashes.len() / 2) {
    //         tracing::debug!("col {}: {:x}", i, commit.hashes[i]);
    //     }
    //
    //     // debug print all the leaves
    //     tracing::debug!("server leaves:");
    //     for hash in server_leaves.iter() {
    //         tracing::debug!("{:x}", hash);
    //     }
    //
    //     tracing::debug!("leaves from file:");
    //     for hash in leaves_from_file.iter() {
    //         tracing::debug!("{:x}", hash);
    //     }
    //
    //     tracing::debug!("streamed file leaves:");
    //     for (hash, col_idx) in streamed_file_leaves.iter().zip(cols_to_verify.iter()) {
    //         tracing::debug!("expected col {}, hash: {:x}", col_idx, hash);
    //     }
    //
    //     assert_eq!(leaves_from_file, server_leaves);
    //     assert_eq!(leaves_from_file, streamed_file_leaves);
    // }

    #[tokio::test]
    #[serial]
    async fn upload_then_download_file() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_download_file".to_string()).await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata = crate::networking::client::get_client_metadata_from_database_by_filename(
            &"test.txt".to_string(),
        ).await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            let delete_result = client::delete_file(
                metadata,
                format!("localhost:{}", port),
            ).await;
        }

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!("file start: {:?}...", file_data.iter().take(10).collect::<Vec<&u8>>());

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        ).await;

        tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        // copy original file to temp location for comparison later
        tokio::fs::copy(source_file, dest_temp_file).await.unwrap();

        tracing::info!("requesting download");
        client::download_file(metadata, format!("localhost:{}", port), 0)
            .await
            .unwrap();

        let mut downloaded_data = fs::read(dest_temp_file).await.unwrap();

        tracing::info!("downloaded file: {:?}", downloaded_data.iter().take(10).collect::<Vec<&u8>>());
        assert_eq!(file_data, downloaded_data);
        tokio::fs::remove_file(dest_temp_file).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_polynomial_evaluation() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_download_file".to_string()).await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata = crate::networking::client::get_client_metadata_from_database_by_filename(
            &"test.txt".to_string(),
        ).await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(
                metadata,
                format!("localhost:{}", port),
            ).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!("file start: {:?}...", file_data.iter().take(10).collect::<Vec<&u8>>());

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        ).await;

        tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        let point = WriteableFt63::from_u128(2);

        let response = client::client_request_and_verify_polynomial::<Blake3>(
            &metadata,
            format!("localhost:{}", port),
        ).await;

        let evaluation_result = response.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_database_creation() {
        let encoded_file_data = random_writeable_field_vec(4);
        let CommitOrLeavesOutput::Commit(commit) = convert_file_data_to_commit(
            &encoded_file_data,
            CommitRequestType::Commit,
            CommitDimensions::Square,
        ).unwrap()
        else { panic!("Unexpected failure to convert file to commitment") };

        let file_metadata = FileMetadata {
            id_ulid: Ulid::new(),
            filename: "a_nonexistent_file".to_string(),
            num_rows: commit.n_rows,
            num_columns: commit.n_per_row,
            num_encoded_columns: commit.n_cols,
            filesize_in_bytes: commit.coeffs.len(),
            stored_server: ServerHost {
                server_name: Some("nonexistent_host".to_string()),
                server_ip: "0.0.0.0".to_string(),
                server_port: 0,
            },
            root: commit.get_root(),
        };

        let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS).await.unwrap();
        db.use_ns(constants::SERVER_NAMESPACE).use_db(constants::SERVER_DATABASE_NAME).await.unwrap();

        db.create::<Option<FileMetadata>>((constants::SERVER_METADATA_TABLE, file_metadata.id_ulid.to_string()))
            // db.create::<Option<FileMetadata>>(constants::SERVER_METADATA_TABLE)
            // db.create::<Option<FileMetadata>>("FileMetadata")
            .content(&file_metadata).await.unwrap();
        tracing::debug!("File metadata appended to database: {:?}", &file_metadata);

        let retrieve_result: Option<FileMetadata> = db.select::<Option<FileMetadata>>((constants::SERVER_METADATA_TABLE, file_metadata.id_ulid.to_string())).await.unwrap();

        // assert_eq!(file_metadata, retrieve_result.unwrap())
    }
}