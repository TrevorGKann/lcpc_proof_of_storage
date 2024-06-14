#[cfg(test)]
pub mod network_tests {
    use std::time::Duration;

    use blake3::Hasher as Blake3;
    use blake3::traits::digest::Output;
    use ff::PrimeField;
    // use pretty_assertions::assert_eq;
    use tokio::fs;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio::time::sleep;

    use crate::fields::writable_ft63::WriteableFt63;
    use crate::lcpc_online::{get_PoS_soudness_n_cols, hash_column_to_digest, server_retreive_columns};
    use crate::networking::client;
    use crate::networking::client::{convert_read_file_to_commit_only_leaves, get_processed_column_leaves_from_file};
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

    fn start_tracing_for_tests() -> Result<(), Box<dyn std::error::Error>> {
        let subscriber = tracing_subscriber::fmt()
            // .pretty()
            .compact()
            .with_file(true)
            .with_line_number(true)
            .with_max_level(tracing::Level::TRACE)
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
    async fn upload_file_test() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_file_test".to_string()).await;

        let response = client::upload_file(
            source_file.to_owned(),
            4,
            format!("localhost:{}", port),
        ).await;

        tracing::info!("client received: {:?}", response);

        let (metadata, root) = response.unwrap();

        assert_eq!(metadata.filename, "test.txt");
        // assert_eq!(metadata.num_columns, 4); //todo uncomment once implemented
        assert_eq!(tokio::fs::read(dest_temp_file).await.unwrap(), tokio::fs::read(source_file).await.unwrap());
    }

    #[tokio::test]
    async fn upload_then_verify() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_verify".to_string()).await;


        let response = client::upload_file(
            source_file.to_owned(),
            4,
            format!("localhost:{}", port),
        ).await;

        tracing::info!("client received: {:?}", response);

        let (metadata, root) = response.unwrap();

        tracing::info!("requesting proof");
        let proof_response = client::request_proof(metadata, format!("localhost:{}", port), 0).await;
        tracing::info!("client received: {:?}", proof_response);

        proof_response.unwrap();
    }

    #[tokio::test]
    async fn test_different_column_hashing_methods_agree() {
        start_tracing_for_tests();
        tracing::info!("starting test_different_column_hashing_methods_agree");

        let test_file = "test_files/test.txt";

        let (root, commit, file_metadata)
            = crate::networking::server::convert_file_to_commit_internal(test_file, None).unwrap();


        let cols_to_verify = crate::networking::client::get_columns_from_random_seed(
            1337,
            // get_PoS_soudness_n_cols(&file_metadata),
            2,
            file_metadata.num_encoded_columns);

        let leaves_from_file = get_processed_column_leaves_from_file(&file_metadata, cols_to_verify.clone()).await;

        let server_columns = server_retreive_columns(&commit, &cols_to_verify);
        let server_leaves: Vec<Output<Blake3>> = server_columns
            .iter()
            .map(hash_column_to_digest::<Blake3>)
            .collect();

        let mut file_data = fs::read(test_file).await.unwrap();
        let streamed_file_leaves = convert_read_file_to_commit_only_leaves::<Blake3>(&file_data, &cols_to_verify).unwrap();

        tracing::debug!("commit's hashes:");
        for i in 0..(commit.hashes.len() / 2) {
            tracing::debug!("col {}: {:x}", i, commit.hashes[i]);
        }

        // debug print all the leaves
        tracing::debug!("server leaves:");
        for hash in server_leaves.iter() {
            tracing::debug!("{:x}", hash);
        }

        tracing::debug!("leaves from file:");
        for hash in leaves_from_file.iter() {
            tracing::debug!("{:x}", hash);
        }

        tracing::debug!("streamed file leaves:");
        for (hash, col_idx) in streamed_file_leaves.iter().zip(cols_to_verify.iter()) {
            tracing::debug!("expected col {}, hash: {:x}", col_idx, hash);
        }

        assert_eq!(leaves_from_file, server_leaves);
        assert_eq!(leaves_from_file, streamed_file_leaves);
    }

    #[tokio::test]
    async fn upload_then_download_file() {
        // setup cleanup files
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_download_file".to_string()).await;

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!("file start: {:?}...", file_data.iter().take(10).collect::<Vec<&u8>>());

        let upload_response = client::upload_file(
            source_file.to_owned(),
            4,
            format!("localhost:{}", port),
        ).await;

        tracing::info!("client received: {:?}", upload_response);

        let (metadata, root) = upload_response.unwrap();

        tracing::info!("requesting download");
        client::download_file(metadata, format!("localhost:{}", port), 0)
            .await
            .unwrap();

        let mut downloaded_data = fs::read(dest_temp_file).await.unwrap();

        tracing::info!("downloaded file: {:?}", downloaded_data.iter().take(10).collect::<Vec<&u8>>());
        assert_eq!(file_data, downloaded_data);
    }

    #[tokio::test]
    async fn test_polynomial_evaluation() {
        let source_file = "test_files/test.txt";
        let dest_temp_file = "test.txt";
        let cleanup = Cleanup { files: vec![dest_temp_file.to_string()] };

        let port = start_test_with_server_on_random_port_and_get_port("upload_then_download_file".to_string()).await;

        let file_data = fs::read(source_file).await.unwrap();
        tracing::info!("file start: {:?}...", file_data.iter().take(10).collect::<Vec<&u8>>());

        let upload_response = client::upload_file(
            source_file.to_owned(),
            4,
            format!("localhost:{}", port),
        ).await;

        tracing::info!("client received: {:?}", upload_response);

        let (metadata, root) = upload_response.unwrap();

        let point = WriteableFt63::from_u128(2);

        let response = client::client_request_and_verify_polynomial::<Blake3>(
            &metadata,
            format!("localhost:{}", port),
        ).await;

        let evaluation_result = response.unwrap();
    }
}