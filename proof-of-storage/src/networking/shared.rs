use blake3::Hasher as Blake3;
use blake3::traits::digest;
use blake3::traits::digest::FixedOutputReset;
use digest::Digest;
use serde::{Deserialize, Serialize};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpStream,
};
use tokio_serde::{formats::Json, Framed};
use lcpc_2d::LcCommit;
use lcpc_ligero_pc::LigeroEncoding;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::{debug, info, instrument, subscriber};
use tracing_subscriber;
use crate::fields;
use crate::fields::writable_ft63::WriteableFt63;



type PoSField = WriteableFt63;
type PoSEncoding = LigeroEncoding<WriteableFt63>;
type PoSCommit = LcCommit<Blake3, PoSEncoding>;
type WrappedStream = FramedRead<OwnedReadHalf, LengthDelimitedCodec>;
type WrappedSink = FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>;

// We use the unit type in place of the message types since we're
// only dealing with one half of the IO
type SerStream<T> = Framed<WrappedStream, T, (), Json<T, ()>>;
type DeSink<T> = Framed<WrappedSink, (), T, Json<(), T>>;
pub(crate) fn wrap_stream<Into, OutOf>(stream: TcpStream) -> (SerStream<OutOf>, DeSink<Into>) {
    let (read, write) = stream.into_split();
    let stream = WrappedStream::new(read, LengthDelimitedCodec::new());
    let sink = WrappedSink::new(write, LengthDelimitedCodec::new());
    // let sink = DeSink::<Into>::new(sink, Json::default());
    (
        SerStream::new(stream, Json::default()),
        DeSink::<Into>::new(sink, Json::default()),
    )
}

type testField = fields::writable_ft63::WriteableFt63;
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessages {
    UserLogin{username: String, password: String},
    UploadNewFile{filename: String, file: Vec<u8>, rows:usize, columns:usize},
    RequestFile{filename: String},
    AppendToFile{filename: String, file: Vec<u8>},
    EditFileRow{filename: String, row: usize, file: Vec<u8>},
    RequestFileRow{filename: String, row: usize},
    RequestEncodedRow{filename: String, row: usize},
    RequestProof{filename: String, columns_to_verify: Vec<usize>},
    RequestPolynomialEvaluation{filename: String, evaluation_point: testField},
}

#[derive(Debug, Serialize, Deserialize)]
// pub enum ServerMessages<D> where D: Digest + FixedOutputReset {
pub enum ServerMessages<T> {
// pub enum ServerMessages{
    UserLoginResponse{success: bool},
    CompactCommit{commit: PoSCommit},
    MerklePathExpansion{merkle_paths: Vec<T>},
    File{file: Vec<u8>},
    EncodedRow{row: Vec<testField>},
    PolynomialEvaluation{evaluation_result: testField},
}