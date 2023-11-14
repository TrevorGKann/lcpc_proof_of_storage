use blake3::traits::digest;
use serde::{Serialize, Deserialize};
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serde::{formats::Json, Serializer, Deserializer};
use lcpc_2d::LcEncoding;

use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
use digest::{Digest, FixedOutputReset, Output};

// async fn send_commitment<D: Digest, E: LcEncoding + ff::PrimeField>(stream: &mut TcpStream, commitment: &LigeroCommit<D, E>) -> Result<(), Box<dyn std::error::Error>> {
//     let mut serializer = Serializer::<_>::new(stream);
//     serializer.serialize(commitment).await?;
//     Ok(())
// }