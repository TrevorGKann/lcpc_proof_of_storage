use std::cmp::min;
use std::time::Duration;

use blake3::Hasher as Blake3;
use blake3::traits::digest;
use digest::Digest;
use serde::{Deserialize, Serialize};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_serde::{Deserializer, Serializer};

use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::fields::read_file_to_field_elements_vec;
use crate::fields::writable_ft63::WriteableFt63;

type PoSField = WriteableFt63;
type PoSEncoding = LigeroEncoding<WriteableFt63>;
type PoSCommit = LcCommit<Blake3, PoSEncoding>;


async fn start_server(ip: &str, port: &str) -> Result<(), Box<dyn std::error::Error>> {
    let listening_IP = format!("{}:{}", ip, port);
    let listener = TcpListener::bind(listening_IP.clone()).await?;
    println!("Server: Server listening on {listening_IP}");

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_client_loop(stream));
    }

    Ok(())
}

async fn handle_client_loop(mut stream: TcpStream){
    println!("Reading line");
    while let Ok(transmission) = read_transmission(&mut stream).await {
        println!("Server: Received transmission: {:?}", transmission);
    }

    println!("Server: Client disconnected");
}



#[derive(Debug, Serialize, Deserialize)]
enum Transmission {
    FileUpload(String),
    FileDownload(String),
    // Add other transmission types as needed
}


async fn read_transmission(stream: &mut TcpStream) -> Result<Transmission, Box<dyn std::error::Error>> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    println!("transmission read!");

    // Deserialize the content of the buffer into the Transmission enum
    let transmission: Transmission = serde_json::from_slice(&buf)?;
    println!("{:?}", transmission);

    Ok(transmission)
}

async fn send_transmission(stream: &mut TcpStream, transmission: &Transmission) -> Result<(), Box<dyn std::error::Error>> {
    // Serialize the Transmission enum and send it over the stream
    let serialized_transmission = serde_json::to_string(transmission)?;
    stream.write_all(serialized_transmission.as_bytes()).await?;

    Ok(())
}

#[tokio::test]
async fn main() {
    // Start the server
    tokio::spawn(async {
        if let Err(e) = start_server("127.0.0.1", "8080").await {
            eprintln!("Server: Server error: {}", e);
        }
    });

    // Wait for the server to start up
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start the client
    let mut client_stream = TcpStream::connect("127.0.0.1:8080").await.unwrap();

    // Example of uploading a file
    let file_upload = Transmission::FileUpload("uploading_file".to_string());
    send_transmission(&mut client_stream, &file_upload).await.unwrap();

    // Example of downloading a file
    let file_download = Transmission::FileDownload("test_file.txt".to_string());
    send_transmission(&mut client_stream, &file_download).await.unwrap();

    // wait 5 seconds
    tokio::time::sleep(Duration::from_secs(2)).await;
}