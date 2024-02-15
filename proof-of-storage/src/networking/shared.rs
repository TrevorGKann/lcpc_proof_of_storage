use blake3::Hasher as Blake3;
use serde::{Deserialize, Serialize};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpStream,
};
use tokio_serde::{formats::Json, Framed};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use lcpc_2d::{LcCommit, LcRoot};
use lcpc_ligero_pc::LigeroEncoding;

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

type TestField = fields::writable_ft63::WriteableFt63;
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessages {
    UserLogin {username: String, password: String},
    UploadNewFile {filename: String, file: Vec<u8>, rows:usize, columns:usize},
    RequestFile {file_metadata: FileMetadata},
    RequestFileRow {file_metadata: FileMetadata, row: usize},
    EditFileRow {file_metadata: FileMetadata, row: usize, file: Vec<u8>},
    AppendToFile {file_metadata: FileMetadata, file: Vec<u8>},
    RequestEncodedColumn {file_metadata: FileMetadata, row: usize},
    RequestProof {file_metadata: FileMetadata, columns_to_verify: Vec<usize>},
    RequestPolynomialEvaluation {file_metadata: FileMetadata, evaluation_point: TestField },
    ClientKeepAlive,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessages<H> where
    H: //this doesn't seem to need anything atm, not even serde
{
    UserLoginResponse {success: bool},
    CompactCommit {root: LcRoot<Blake3,  LigeroEncoding<WriteableFt63>>, file_metadata: FileMetadata},
    MerklePathExpansion {merkle_paths: Vec<H>},
    File {file: Vec<u8>},
    FileRow {row: Vec<u8>},
    EncodedColumn {col: Vec<TestField>},
    PolynomialEvaluation {evaluation_result: TestField },
    ServerKeepAlive,
    BadResponse {error: String},
}

//todo: these should probably be kept in a database internally to the server to keep track of the files
#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub filename: String,
    pub rows: usize,
    pub encoded_columns: usize,
    pub filesize_in_bytes: usize,
}

impl FileMetadata {
    pub fn get_file_columns(&self) -> usize {
        self.encoded_columns / 2
    }

    pub fn get_end_coordinates(&self) -> (usize, usize) {
        (self.rows, self.filesize_in_bytes % self.encoded_columns)
    }
}