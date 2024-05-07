#[cfg(test)]
pub mod network_tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use tokio::net::TcpListener;
    use tokio::time::sleep;

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

    fn test_start_tracing() -> Result<(), Box<dyn std::error::Error>> {
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

    async fn start_test(test_name: String) -> u16 {
        test_start_tracing();
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

        let port = start_test("upload_file_test".to_string()).await;

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

        let port = start_test("upload_then_verify".to_string()).await;


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
}