use std::time::Duration;

use blake3::Hasher as Blake3;
use blake3::traits::digest;
use digest::Digest;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_serde::{Deserializer, Serializer};

use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::fields::read_file_to_field_elements_vec;
use crate::fields::writable_ft63::WriteableFt63;

#[derive(Debug, Serialize, Deserialize)]
pub enum NextMessage {
    //client messages
    FileRow{row_number: usize},
    Challenge,

    //server messages
    Commit,
    Response,
}


async fn handle_client(mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    // Read the file from the client
    let mut file: File = read_message(&mut stream).await?;

    // Process the file and generate a commitment
    let commitment = process_file(&mut file);

    // Send the commitment to the client
    send_message(&mut stream, &commitment).await?;

    // Interactive proof: Repeat as needed
    loop {
        // Read the challenge from the client
        let challenge: Challenge = read_message(&mut stream).await?;

        // Generate a response to the challenge
        let response = generate_proof_to_challenge(challenge);

        // Send the response to the client
        send_message(&mut stream, &response).await?;

        // Repeat the loop based on your protocol conditions
        // You might have a termination condition based on the challenge
    }
}


async fn send_file_and_receive_commitment<D: Digest, E: LcEncoding>(file: File) -> Result<LcCommit<D,E>, Box<dyn std::error::Error>> {
    // Connect to the server
    let mut stream = TcpStream::connect("127.0.0.1:8080").await?;

    // Send the file to the server
    send_message(&mut stream, &file).await?;

    // Receive the commitment from the server
    let commitment: LcCommit<D,E> = read_message(&mut stream).await?;

    Ok(commitment)
}


fn process_file<D: Digest, E: LcEncoding>(file: &mut File) -> LcCommit<D, E> {
    // Process the file and generate a commitment
    // Replace this with your actual logic
    let data: Vec<WriteableFt63> = read_file_to_field_elements_vec(file .into());
    let data_min_width = (data.len() as f32).sqrt().ceil() as usize;
    let data_realized_width = data_min_width.next_power_of_two();
    let matrix_columns = (data_realized_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_columns);
    let comm = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
    return comm;
}

// Your application-specific logic goes here
#[derive(Debug, Serialize, Deserialize)]
struct Challenge {
    // Your challenge structure goes here
    // Example:
    // challenge_data: Vec<u8>,
}

// Your application-specific logic goes here
fn generate_proof_to_challenge(challenge: Challenge) -> Response {
    // Generate a response to the challenge
    // Replace this with your actual logic
    Response { /* Your response logic here */ }
}

// Your application-specific logic goes here
#[derive(Debug, Serialize, Deserialize)]
struct Response {
    // Your response structure goes here
    // Example:
    // response_data: Vec<u8>,
}

#[tokio::test]
async fn main() {
    // Start the server
    tokio::spawn(async {
        if let Err(e) = start_server().await {
            eprintln!("Server error: {}", e);
        }
    });

    // Give the server some time to start
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create a file for testing
    let file = File::open("test_file.txt").await.unwrap();

    // Send the file to the server and receive the commitment
    let response_result = send_file_and_receive_commitment(file).await?;

    // generate challenge and request response from server

}

async fn start_server() -> Result<(), Box<dyn std::error::Error>> {
    let listening_IP = "127.0.0.1:8080";
    let listener = TcpListener::bind(listening_IP).await?;
    println!("Server listening on {listening_IP}");

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_client(stream));
    }

    Ok(())
}

pub async fn read_message<T>(stream: &mut TcpStream) -> Result<T, Box<dyn std::error::Error>>
    where
        T: for<'de> Deserialize<'de>,
{
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let message: T = serde_json::from_slice(&buf)?;
    Ok(message)
}

// Helper function to send a message to the stream
pub async fn send_message<T>(stream: &mut TcpStream, message: &T) -> Result<(), Box<dyn std::error::Error>>
    where
        T: Serialize,
{
    let serialized_message = serde_json::to_string(message)?;
    stream.write_all(serialized_message.as_bytes()).await?;
    Ok(())
}

