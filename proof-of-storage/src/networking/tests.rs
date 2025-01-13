#![feature(generic_const_exprs)]
use crate::networking::client;

#[cfg(test)]
pub mod network_tests {
    use std::env;
    use std::io::SeekFrom;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use anyhow::{bail, ensure, Result};
    use blake3::traits::digest::Output;
    use blake3::Hasher as Blake3;
    use ff::PrimeField;
    // use pretty_assertions::assert_eq;
    use metrics::histogram;
    use num_traits::pow;
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    use rayon::iter::IntoParallelRefIterator;
    use serial_test::serial;
    use surrealdb::engine::local::RocksDb;
    use surrealdb::Surreal;
    use tokio::fs;
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Instant};
    use tracing::{span, Instrument, Level, Span};
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::EnvFilter;
    use ulid::Ulid;

    use crate::databases::*;
    use crate::fields;
    use crate::fields::WriteableFt63;
    use crate::fields::{convert_byte_vec_to_field_elements_vec, random_writeable_field_vec};
    use crate::lcpc_online::{
        convert_file_data_to_commit, decode_row,
        form_side_vectors_for_polynomial_evaluation_from_point, get_PoS_soudness_n_cols,
        hash_column_to_digest, server_retreive_columns, verifiable_polynomial_evaluation,
        verify_full_polynomial_evaluation_wrapper_with_single_eval_point, CommitDimensions,
        CommitOrLeavesOutput, CommitRequestType,
    };
    use crate::networking::client;
    use crate::networking::server::handle_client_loop;
    use crate::tests::tests::Cleanup;

    use super::*;

    #[tracing::instrument(name = "Server thread", parent = None)]
    async fn test_start_server(port: u16) {
        tracing::info!("Server starting");

        let listening_address = format!("0.0.0.0:{}", port);

        let listener = TcpListener::bind(listening_address)
            .await
            .map_err(|e| tracing::error!("server failed to start: {:?}", e))
            .expect("failed to initialize listener");

        tracing::info!(
            "Server started on port {:?}",
            listener.local_addr().unwrap()
        );

        // let server_span = span!(parent: None, Level::INFO, "Server thread");

        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(
                async move {
                    let connection_span = span!(Level::INFO, "client connection");
                    let _enter = connection_span.enter();
                    handle_client_loop(stream).in_current_span().await
                }
                .in_current_span(),
            );
        }
    }

    fn start_tracing_for_tests() -> Result<()> {
        let env_filter: EnvFilter = EnvFilter::builder()
            .parse("surrealdb_core=warn,surrealdb=warn,trace")
            .unwrap();

        let subscriber = tracing_subscriber::fmt()
            .pretty()
            .compact()
            .with_file(true)
            .with_line_number(true)
            .with_max_level(tracing::Level::TRACE)
            .with_env_filter(env_filter)
            // .with_span_events(FmtSpan::FULL)
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
        tokio::spawn(test_start_server(port));
        port
    }

    #[tokio::test]
    #[serial]
    async fn upload_and_delete_file_test() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        // let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port =
            start_test_with_server_on_random_port_and_get_port("upload_file_test".to_string())
                .await;

        // upload the file
        let response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        tracing::debug!("client received: {:?}", response);

        let metadata = response.unwrap();

        assert_eq!(metadata.filename, source_file);
        assert_eq!(metadata.num_columns, 4); //todo uncomment once implemented
        assert_eq!(metadata.num_encoded_columns, 8);

        tracing::debug!("client requesting file deletion");
        client::delete_file(metadata, format!("localhost:{}", port))
            .await
            .unwrap();
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn upload_then_verify() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port =
            start_test_with_server_on_random_port_and_get_port("upload_then_verify".to_string())
                .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        tracing::debug!("client received: {:?}", response);

        let metadata = response.unwrap();

        tracing::info!("requesting proof");
        let proof_response =
            client::request_proof(metadata, format!("localhost:{}", port), 0).await;
        tracing::debug!("client received: {:?}", proof_response);

        proof_response.unwrap();
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn upload_then_download_file() {
        // setup cleanup files
        let source_file = "test_files/4000_byte_file.bytes";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!(
            "file start: {:?}...",
            file_data.iter().take(10).collect::<Vec<&u8>>()
        );
        // copy original file to temp location for comparison later
        tokio::fs::copy(source_file, dest_temp_file).await.unwrap();

        let port = start_test_with_server_on_random_port_and_get_port(
            "upload_then_download_file".to_string(),
        )
        .await;

        let client_span = span!(Level::INFO, "Client thread").entered();
        let start = Instant::now();
        let histogram = histogram!("client_time");

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        // tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        tracing::info!("requesting download");
        client::download_file(metadata, format!("localhost:{}", port), 0)
            .await
            .unwrap();

        histogram.record(start.elapsed());
        client_span.exit();

        let mut downloaded_data = fs::read(dest_temp_file).await.unwrap();

        tracing::info!(
            "downloaded file: {:?}",
            downloaded_data.iter().take(10).collect::<Vec<&u8>>()
        );
        assert_eq!(file_data, downloaded_data);
        tokio::fs::remove_file(dest_temp_file).await.unwrap();
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_polynomial_evaluation() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port = start_test_with_server_on_random_port_and_get_port(
            "upload_then_download_file".to_string(),
        )
        .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!(
            "file start: {:?}...",
            file_data.iter().take(10).collect::<Vec<&u8>>()
        );

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        let point = WriteableFt63::from_u128(2);

        let response = client::client_request_and_verify_polynomial::<Blake3>(
            &metadata,
            format!("localhost:{}", port),
        )
        .await;

        let evaluation_result = response.unwrap();
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_database_creation() {
        let encoded_file_data = random_writeable_field_vec(4);
        let CommitOrLeavesOutput::Commit(commit) = convert_file_data_to_commit(
            &encoded_file_data,
            CommitRequestType::Commit,
            CommitDimensions::Square,
        )
        .unwrap() else {
            panic!("Unexpected failure to convert file to commitment")
        };

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

        let db = Surreal::new::<RocksDb>(constants::DATABASE_ADDRESS)
            .await
            .unwrap();
        db.use_ns(constants::SERVER_NAMESPACE)
            .use_db(constants::SERVER_DATABASE_NAME)
            .await
            .unwrap();

        db.create::<Option<FileMetadata>>((
            constants::SERVER_METADATA_TABLE,
            file_metadata.id_ulid.to_string(),
        ))
        // db.create::<Option<FileMetadata>>(constants::SERVER_METADATA_TABLE)
        // db.create::<Option<FileMetadata>>("FileMetadata")
        .content(&file_metadata)
        .await
        .unwrap();
        tracing::debug!("File metadata appended to database: {:?}", &file_metadata);

        let retrieve_result: Option<FileMetadata> = db
            .select::<Option<FileMetadata>>((
                constants::SERVER_METADATA_TABLE,
                file_metadata.id_ulid.to_string(),
            ))
            .await
            .unwrap();

        // assert_eq!(file_metadata, retrieve_result.unwrap())
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    async fn test_polynomial_evaluations_with_different_methods() {
        use ff::Field;
        use PrimeField;
        start_tracing_for_tests();
        tracing::info!("Starting test_polynomial_evaluations_with_different_methods");

        let mut random_seed = ChaCha8Rng::seed_from_u64(1337);
        let source_polynomial = fields::random_writeable_field_vec(5);
        let evaluation_point = WriteableFt63::random(&mut random_seed);

        let expected_result =
            fields::evaluate_field_polynomial_at_point(&source_polynomial, &evaluation_point);

        let CommitOrLeavesOutput::Commit::<Blake3, _>(tall_commitment) =
            convert_file_data_to_commit(
                &source_polynomial,
                CommitRequestType::Commit,
                CommitDimensions::Specified {
                    num_pre_encoded_columns: 4,
                    num_encoded_columns: 8,
                },
            )
            .unwrap()
        else {
            panic!()
        };

        let (tall_evaluation_left_vector, tall_evaluation_right_vector) =
            form_side_vectors_for_polynomial_evaluation_from_point(
                &evaluation_point,
                tall_commitment.n_rows,
                tall_commitment.n_per_row,
            );
        let tall_result_vector =
            verifiable_polynomial_evaluation(&tall_commitment, &tall_evaluation_left_vector);
        let decoded_tall_result_vector = decode_row(tall_result_vector.to_vec()).unwrap();
        tracing::debug!("Evaluation point: {:?}", &evaluation_point);
        tracing::debug!("tall_result_vector: {:?}", &decoded_tall_result_vector);
        tracing::debug!(
            "tall_evaluation_right_vector: {:?}",
            &tall_evaluation_right_vector
        );
        tracing::debug!(
            "tall_evaluation_left_vector: {:?}",
            &tall_evaluation_left_vector
        );
        let tall_result =
            fields::vector_multiply(&decoded_tall_result_vector, &tall_evaluation_right_vector);

        let CommitOrLeavesOutput::Commit::<Blake3, _>(wide_commitment) =
            convert_file_data_to_commit(
                &source_polynomial,
                CommitRequestType::Commit,
                CommitDimensions::Specified {
                    num_pre_encoded_columns: 8,
                    num_encoded_columns: 16,
                },
            )
            .unwrap()
        else {
            panic!()
        };

        let (wide_evaluation_left_vector, wide_evaluation_right_vector) =
            form_side_vectors_for_polynomial_evaluation_from_point(
                &evaluation_point,
                wide_commitment.n_rows,
                wide_commitment.n_per_row,
            );
        let wide_result_vector =
            verifiable_polynomial_evaluation(&wide_commitment, &wide_evaluation_left_vector);
        let decoded_wide_result_vector = decode_row(wide_result_vector.to_vec()).unwrap();
        tracing::debug!("wide_result_vector: {:?}", &decoded_wide_result_vector);
        tracing::debug!(
            "wide_evaluation_right_vector: {:?}",
            &wide_evaluation_right_vector
        );
        tracing::debug!(
            "wide_evaluation_left_vector: {:?}",
            &wide_evaluation_left_vector
        );
        let wide_result =
            fields::vector_multiply(&decoded_wide_result_vector, &wide_evaluation_right_vector);

        tracing::debug!("expected_result: {:?}", expected_result);
        tracing::debug!("tall_result: {:?}", tall_result);
        tracing::debug!("wide_result: {:?}", wide_result);

        assert_eq!(expected_result, tall_result);
        assert_eq!(expected_result, wide_result);
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_metadata_reshape() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port =
            start_test_with_server_on_random_port_and_get_port("test_metadata_reshape".to_string())
                .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let file_data = fs::read(source_file).await.unwrap();

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        let reshaped_metadata =
            client::reshape_file::<Blake3>(&metadata, format!("localhost:{}", port), 128, 8, 16)
                .await
                .unwrap();

        assert_eq!(reshaped_metadata.num_columns, 8);
        assert_eq!(reshaped_metadata.num_encoded_columns, 16);
        assert_eq!(reshaped_metadata.filename, metadata.filename);
        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_file_append() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port =
            start_test_with_server_on_random_port_and_get_port("test_file_append".to_string())
                .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let file_data = fs::read(source_file).await.unwrap();

        let upload_response = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;

        tracing::debug!("client received: {:?}", upload_response);

        let metadata = upload_response.unwrap();

        let mut data_to_append = fs::read(source_file).await.unwrap();

        let appended_response = client::append_to_file(
            metadata.clone(),
            format!("localhost:{}", port),
            8,
            data_to_append.clone(),
        )
        .await;

        tracing::debug!("client received: {:?}", appended_response);

        let appended_metadata = appended_response.unwrap();

        assert_eq!(appended_metadata.num_columns, 4);
        assert_eq!(appended_metadata.num_encoded_columns, 8);
        assert_eq!(appended_metadata.filename, metadata.filename);
        assert!(appended_metadata.num_rows >= metadata.num_rows);
        assert_eq!(
            appended_metadata.filesize_in_bytes,
            metadata.filesize_in_bytes * 2
        );

        tokio::fs::rename(source_file, dest_temp_file)
            .await
            .unwrap();

        let download_response =
            client::download_file(appended_metadata, format!("localhost:{}", port), 32).await;
        tracing::debug!("client received: {:?}", download_response);
        download_response.unwrap();

        let downloaded_data = fs::read(source_file).await.unwrap();
        let mut appended_data = data_to_append.clone();
        appended_data.append(&mut data_to_append);

        assert_eq!(downloaded_data, appended_data);

        tokio::fs::rename(dest_temp_file, source_file)
            .await
            .unwrap();

        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_file_edit() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port =
            start_test_with_server_on_random_port_and_get_port("test_file_append".to_string())
                .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let metadata = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await;
        tracing::debug!("client received: {:?}", metadata);
        let metadata = metadata.unwrap();

        // generate a vector of random bytes to edit in
        let mut expected_data = fs::read(source_file).await.unwrap();
        let mut seed = ChaCha8Rng::seed_from_u64(1337);
        let mut random_data = vec![0u8; 100];
        seed.fill_bytes(&mut random_data);
        let location_to_edit_to =
            seed.next_u32() as usize % (expected_data.len() - random_data.len());
        expected_data.splice(
            location_to_edit_to..location_to_edit_to + random_data.len(),
            random_data.clone().into_iter(),
        );

        tracing::debug!("client requesting file edit");
        let edit_metadata = client::edit_file(
            metadata.clone(),
            format!("localhost:{}", port),
            8,
            random_data.clone(),
            location_to_edit_to,
        )
        .await;
        tracing::debug!("client received: {:?}", edit_metadata);
        let edit_metadata = edit_metadata.unwrap();

        assert_eq!(edit_metadata.num_columns, 4);
        assert_eq!(edit_metadata.num_encoded_columns, 8);
        assert_eq!(edit_metadata.filename, metadata.filename);
        assert_eq!(edit_metadata.num_rows, metadata.num_rows);
        assert_eq!(edit_metadata.filesize_in_bytes, metadata.filesize_in_bytes);

        tokio::fs::rename(source_file, dest_temp_file)
            .await
            .unwrap();

        let download_response =
            client::download_file(edit_metadata, format!("localhost:{}", port), 32).await;
        tracing::debug!("client received: {:?}", download_response);
        download_response.unwrap();

        let downloaded_data = fs::read(source_file).await.unwrap();

        assert_eq!(downloaded_data, expected_data);

        tokio::fs::rename(dest_temp_file, source_file)
            .await
            .unwrap();

        tracing::info!("Test passed successfully!");
    }

    #[tokio::test]
    #[serial]
    async fn test_file_verification_rejects_bad_proofs() {
        use std::env;
        use tokio::fs::OpenOptions;

        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup {
            files: vec![dest_temp_file.to_string()],
        };

        let port =
            start_test_with_server_on_random_port_and_get_port("test_file_append".to_string())
                .await;

        // try to delete the file first, in case it's already uploaded
        let to_delete_metadata =
            crate::networking::client::get_client_metadata_from_database_by_filename(
                &"test.txt".to_string(),
            )
            .await;

        if let Ok(Some(metadata)) = to_delete_metadata {
            tracing::debug!("client requesting file deletion");
            let delete_result = client::delete_file(metadata, format!("localhost:{}", port)).await;
            tracing::debug!("client received: {:?}", delete_result);
        } else {
            tracing::debug!("client did not request file deletion, no file found on database");
        }

        let metadata = client::upload_file(
            source_file.to_owned(),
            Some(4),
            Some(8),
            format!("localhost:{}", port),
        )
        .await
        .unwrap();

        assert_eq!(metadata.num_columns, 4);
        assert_eq!(metadata.num_encoded_columns, 8);
        assert_eq!(metadata.filename, source_file);
        assert_eq!(
            metadata.filesize_in_bytes,
            tokio::fs::read(source_file).await.unwrap().len()
        );

        client::request_proof(metadata.clone(), format!("localhost:{}", port), 8)
            .await
            .unwrap();

        // Modify the file on the server end, this should make the client fail its next test
        {
            let mut path = env::current_dir().unwrap();
            path.push(constants::SERVER_FILE_FOLDER);

            path.push(format!(
                "{}.{}",
                metadata.id_ulid.to_string(),
                constants::FILE_EXTENSION
            ));

            let mut servers_file = OpenOptions::new().write(true).open(&path).await.unwrap();

            let mut seed = ChaCha8Rng::seed_from_u64(1337);
            let seek_point = seed.next_u32() as usize % (metadata.filesize_in_bytes - 2);
            let mut bytes_to_edit = vec![0u8; 2];
            seed.fill_bytes(&mut bytes_to_edit);

            servers_file
                .seek(SeekFrom::Start(seek_point as u64))
                .await
                .unwrap();
            servers_file.write_all(&bytes_to_edit).await.unwrap();
            servers_file.flush().await.unwrap();
        }

        let bad_verify_result =
            client::request_proof(metadata, format!("localhost:{}", port), 8).await;
        tracing::debug!("client received: {:?}", bad_verify_result);

        assert!(bad_verify_result.is_err());
        tracing::info!("Test passed successfully!");
    }

    #[ignore]
    #[tokio::test]
    async fn generate_test_files_of_different_sizes() {
        use tokio::io::AsyncWriteExt;

        let prefixes = vec![1usize, 4];
        let orders_of_magnitude: Vec<usize> = (3..=9).collect();
        let sizes: Vec<usize> = vec![];

        let mut test_files_path = env::current_dir().unwrap();
        test_files_path.push("test_files");

        let mut rng = rand::thread_rng();
        let mut buffer = [0u8; 8192];

        for order_mag in orders_of_magnitude.iter() {
            for prefix in prefixes.iter() {
                let total_size = pow(10, *order_mag) * prefix;

                let mut file_location = test_files_path.clone();
                file_location.push(format!("{}_byte_file.bytes", total_size));
                println!("creating the file {}", file_location.display());
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(&file_location)
                    .await
                    .unwrap();

                let mut bytes_written = 0;
                let mut bytes_left = total_size;

                while bytes_left > 0 {
                    let chunk_size = bytes_left.min(buffer.len());
                    rng.fill_bytes(&mut buffer[0..chunk_size]);
                    file.write_all(&buffer[..chunk_size]).await.unwrap();
                    bytes_written += chunk_size;
                    bytes_left -= chunk_size;
                }
            }
        }
    }
}
