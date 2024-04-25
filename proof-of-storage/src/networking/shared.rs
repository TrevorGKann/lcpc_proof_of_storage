use blake3::Hasher as Blake3;
use serde::{Deserialize, Serialize};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpStream,
};
use tokio_serde::{formats::Json, Framed};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use lcpc_2d::LcRoot;
use lcpc_ligero_pc::LigeroEncoding;

use crate::{PoSColumn, PoSField};
use crate::fields::writable_ft63::WriteableFt63;
use crate::file_metadata::ClientOwnedFileMetadata;

type WrappedStream = FramedRead<OwnedReadHalf, LengthDelimitedCodec>;
type WrappedSink = FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>;

// We use the unit type in place of the message types since we're
// only dealing with one half of the IO
pub type SerStream<T> = Framed<WrappedStream, T, (), Json<T, ()>>;
pub type DeSink<T> = Framed<WrappedSink, (), T, Json<(), T>>;

//todo need to flip order of return values to match generics
pub(crate) fn wrap_stream<Into, OutOf>(stream: TcpStream) -> (SerStream<OutOf>, DeSink<Into>) {
    let (read, write) = stream.into_split();
    let stream = WrappedStream::new(read, LengthDelimitedCodec::new());
    let sink = WrappedSink::new(write, LengthDelimitedCodec::new());

    (
        SerStream::<OutOf>::new(stream, Json::default()),
        DeSink::<Into>::new(sink, Json::default()),
    )
}

pub struct TypedFramedWrapper<Into, OutOf> {
    pub stream: SerStream<OutOf>,
    pub sink: DeSink<Into>,
    pub server_ip: String,
}
pub(crate) async fn wrap_stream_from_ip<Into, OutOf>(server_ip: String) -> TypedFramedWrapper<Into, OutOf> {

    let mut stream = TcpStream::connect(&server_ip).await.unwrap();
    let (read, write) = stream.into_split();
    let stream = WrappedStream::new(read, LengthDelimitedCodec::new());
    let sink = WrappedSink::new(write, LengthDelimitedCodec::new());

    TypedFramedWrapper {
        stream: SerStream::<OutOf>::new(stream, Json::default()),
        sink: DeSink::<Into>::new(sink, Json::default()),
        server_ip,
    }

}

type TestField = PoSField;

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessages {
    UserLogin { username: String, password: String },
    UploadNewFile { filename: String, file: Vec<u8>, columns: usize },
    RequestFile { file_metadata: ClientOwnedFileMetadata },
    RequestFileRow { file_metadata: ClientOwnedFileMetadata, row: usize },
    EditFileRow { file_metadata: ClientOwnedFileMetadata, row: usize, file: Vec<u8> },
    AppendToFile { file_metadata: ClientOwnedFileMetadata, file: Vec<u8> },
    RequestEncodedColumn { file_metadata: ClientOwnedFileMetadata, row: usize },
    RequestProof { file_metadata: ClientOwnedFileMetadata, columns_to_verify: Vec<u64> },
    RequestPolynomialEvaluation { file_metadata: ClientOwnedFileMetadata, evaluation_point: TestField },
    ClientKeepAlive,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessages
{
    UserLoginResponse { success: bool },
    CompactCommit { root: LcRoot<Blake3, LigeroEncoding<WriteableFt63>>, file_metadata: ClientOwnedFileMetadata },
    MerklePathExpansion { merkle_paths: Vec<PoSColumn> },
    File { file: Vec<u8> },
    FileRow { row: Vec<u8> },
    EncodedColumn { col: Vec<TestField> },
    PolynomialEvaluation { evaluation_result: TestField },
    ServerKeepAlive,
    BadResponse { error: String },
}