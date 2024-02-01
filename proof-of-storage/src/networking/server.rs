use blake3::traits::digest;
use serde::{Serialize, Deserialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serde::{formats::Json, Serializer, Deserializer};
use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
use digest::{Digest, FixedOutputReset, Output};
use tokio::fs::File;
use crate::networking::shared;

use crate::networking::shared::*;


#[derive(Serialize, Deserialize, Debug)]
struct MyMessage {
    field: String,
}
#[tokio::test]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_line_number(false)
        .finish();
    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber)?;


    let listener = TcpListener::bind("0.0.0.0:8080").await
        .map_err(|e| tracing::error!("server failed to start: {:?}", e))
        .expect("failed to initialize listener");

    tokio::spawn(async move {
        //server logic

        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {handle_client_loop(stream).await});
        }
    });


    // client logic
    let stream = TcpStream::connect("0.0.0.0:8080").await?;
    let (mut stream, mut sink) = wrap_stream::<ClientMessages,ServerMessages<MyStruct>>(stream);

    sink.send(ClientMessages::UserLogin {username: "trevor".to_owned(), password: "password".to_owned()})
        .await.expect("Failed to send message to server");

    let Some(Ok(transmission)) = stream.next().await else {
        tracing::error!("Failed to receive message from server");
        return core::result::Result::Err(Box::from("Failed to receive message from server"));
    };
    tracing::info!("Client received: {:?}", transmission);

    Ok(())
}

use blake3::Hasher as Blake3;
use futures::{SinkExt, StreamExt};
use crate::fields;
use crate::fields::writable_ft63::WriteableFt63;

async fn handle_client_loop(mut stream: TcpStream) {
    let (mut stream, mut sink) = wrap_stream::<ServerMessages<MyStruct>, ClientMessages>(stream);
    while let Some(Ok(transmission)) = stream.next().await {
        tracing::info!("Server received: {:?}", transmission);
        match transmission {
            ClientMessages::UserLogin { username, password } => {
                sink.send(ServerMessages::UserLoginResponse { success: true })
                    .await
                    .expect("Failed to send message to client");
            }
            _ => {}
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct MyStruct {
    field_element: crate::fields::writable_ft63::WriteableFt63,
}