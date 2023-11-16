use blake3::traits::digest;
use serde::{Serialize, Deserialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serde::{formats::Json, Serializer, Deserializer};
use lcpc_2d::{LcCommit, LcEncoding};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
use digest::{Digest, FixedOutputReset, Output};
use tokio::fs::File;

use crate::networking::shared::*;

