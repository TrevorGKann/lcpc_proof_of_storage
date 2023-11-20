use std::cmp::min;
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
    StartFileUpload{file_name: String, row_count: usize, row_size: usize},
    FileRow{row_number: usize},
    RequestCommit,
    Challenge,
    EndTransmissions,

    //server messages
    Ready,
    Commit,
    Response,
}

type PoSField = WriteableFt63;
type PoSEncoding = LigeroEncoding<WriteableFt63>;
type PoSCommit = LcCommit<Blake3, PoSEncoding>;

pub struct ServerStatePerConnection {
    file_name: String,
    row_count: usize,
    row_size: usize,
    row_number: usize,
    encoding:  Option<Box<PoSEncoding>>,
    commitment: Option<Box<PoSCommit>>,
}


async fn handle_client_loop(mut stream: TcpStream)  {

    let state = ServerStatePerConnection {
        file_name: String::new(),
        row_count: 0,
        row_size: 0,
        row_number: 0,
        encoding: None,
        commitment: None,
    };

    loop {

        let next_message_type: NextMessage = read_message(&mut stream).await.unwrap_or(NextMessage::EndTransmissions);

        match next_message_type {
            NextMessage::StartFileUpload{file_name, row_count, row_size} => {
                println!("StartFileUpload: file_name: {}, row_count: {}, row_size: {}", file_name, row_count, row_size);
                download_file(&mut stream, &file_name, row_count, row_size).await.unwrap();
            },
            NextMessage::FileRow{row_number} => {
                println!("FileRow: row_number: {}", row_number);
            },
            NextMessage::RequestCommit => {
                println!("RequestCommit");
            },
            NextMessage::Challenge => {
                println!("Challenge");
            },
            NextMessage::EndTransmissions => {
                println!("EndTransmissions");
            },
            _ => {
                println!("Client should not send a server message type");
            }
        }

    }
}


async fn download_file(mut stream: &TcpStream, file_name: &String, row_count: usize, row_size: usize) -> Result<(), dyn std::error::Error> {
    let file_name = format!("{}.download", file_name);
    let new_file = File::create(&file_name).unwrap();
    let mut file = File::open(&file_name).unwrap();
    let mut lines_read_so_far = 0;
    while lines_read_so_far < row_count {
        let read_line : Vec<u8> = read_message(&mut stream).await.unwrap();
        file.write_all(&read_line).await.unwrap();
        lines_read_so_far += 1;
    }

    file.close().await.unwrap();
    return Ok(())
}

#[tokio::test]
async fn main() {
    // Start the server
    tokio::spawn(async {
        if let Err(e) = start_server("127.0.0.1", "8080").await {
            eprintln!("Server error: {}", e);
        }
    });

    // Wait for the server to start up
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Upload a file
    upload_file( "127.0.0.1", "8080","test_file.txt").await;

}

async fn start_server(ip: &str, port: &str) -> Result<(), Box<dyn std::error::Error>> {
    let listening_IP = format!("{}:{}", ip, port);
    let listener = TcpListener::bind(listening_IP.clone()).await?;
    println!("Server listening on {listening_IP}");

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_client_loop(stream));
    }

    Ok(())
}

async fn upload_file(ip: &str, port: &str, file_name: &str) {
    let mut stream = TcpStream::connect(format!("{}:{}", ip, port)).await.unwrap();
    let mut file = File::open(file_name).await.unwrap();
    let mut data_buffer = Vec::new();
    file.read_to_end(&mut data_buffer).await.unwrap();
    let data_width = (data_buffer.len() as f32).sqrt().ceil() as usize;
    send_message(&stream, &NextMessage::StartFileUpload {
        file_name: file_name.to_string(),
        row_count: data_width,
        row_size: data_width,
    }).await.unwrap();

    //iterate through chunks to send in the size of row_size
    let mut chunk_start = 0;
    while chunk_start < data_buffer.len() {
        let chunk_end = chunk_start + data_width;
        let chunk = &data_buffer[chunk_start..min(chunk_end, data_buffer.len())];
        send_message(&stream, &chunk).await.unwrap();
        chunk_start = chunk_end;
    }
}



// Helper Functions for sending and receiving serialized structs to the stream

pub async fn read_message<T>(stream: &mut TcpStream) -> Result<T, Box<dyn std::error::Error>>
    where
        T: for<'de> Deserialize<'de>,
{
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let message: T = serde_json::from_slice(&buf)?;
    Ok(message)
}

pub async fn send_message<T>(mut stream: &TcpStream, message: &T) -> Result<(), Box<dyn std::error::Error>>
    where
        T: Serialize,
{
    let serialized_message = serde_json::to_string(message)?;
    stream.write_all(serialized_message.as_bytes()).await?;
    Ok(())
}

