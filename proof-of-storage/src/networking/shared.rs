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
use crate::databases::FileMetadata;
use crate::fields::writable_ft63::WriteableFt63;

type WrappedStream = FramedRead<OwnedReadHalf, LengthDelimitedCodec>;
type WrappedSink = FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>;

// We use the unit type in place of the message types since we're
// only dealing with one half of the IO
pub type SerStream<T> = Framed<WrappedStream, T, (), Json<T, ()>>;
pub type DeSink<T> = Framed<WrappedSink, (), T, Json<(), T>>;

//todo ought to flip order of return values to match generics
pub(crate) fn wrap_stream<Into, OutOf>(stream: TcpStream) -> (SerStream<OutOf>, DeSink<Into>) {
    let (read, write) = stream.into_split();
    let stream = WrappedStream::new(read, LengthDelimitedCodec::new());
    let sink = WrappedSink::new(write, LengthDelimitedCodec::new());

    (
        SerStream::<OutOf>::new(stream, Json::default()),
        DeSink::<Into>::new(sink, Json::default()),
    )
}

type TestField = PoSField;

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessages {
    NewUser { username: String, password: String },
    UserLogin { username: String, password: String },
    UploadNewFile { filename: String, file: Vec<u8>, columns: usize, encoded_columns: usize },
    RequestFile { file_metadata: FileMetadata },
    RequestFileRow { file_metadata: FileMetadata, row: usize },
    EditFileRow { file_metadata: FileMetadata, row: usize, file: Vec<u8> },
    AppendToFile { file_metadata: FileMetadata, append_data: Vec<u8> },
    RequestEncodedColumn { file_metadata: FileMetadata, row: usize },
    RequestProof { file_metadata: FileMetadata, columns_to_verify: Vec<usize> },
    RequestPolynomialEvaluation { file_metadata: FileMetadata, evaluation_point: TestField },
    RequestFileReshape {
        file_metadata: FileMetadata,
        new_pre_encoded_columns: usize,
        new_encoded_columns: usize,
    },
    RequestReshapeEvaluation {
        old_file_metadata: FileMetadata,
        new_file_metadata: FileMetadata,
        evaluation_point: TestField,
        columns_to_expand_original: Vec<usize>,
        columns_to_expand_new: Vec<usize>,
    },
    RequestEditOrAppendEvaluation {
        old_file_metadata: FileMetadata,
        new_file_metadata: FileMetadata,
        evaluation_point: TestField,
        columns_to_expand: Vec<usize>,
        // functionally equivalent of ReshapeEvaluation but with the omission of differing columns to expand
    },
    ReshapeResponse {
        new_file_metadata: FileMetadata,
        old_file_metadata: FileMetadata,
        accepted: bool,
    },
    EditOrAppendResponse {
        new_file_metadata: FileMetadata,
        old_file_metadata: FileMetadata,
        accepted: bool,
    },
    DeleteFile { file_metadata: FileMetadata },
    ClientKeepAlive,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessages
{
    UserLoginResponse { success: bool },
    CompactCommit { file_metadata: FileMetadata },
    Columns { columns: Vec<PoSColumn> },
    File { file: Vec<u8> },
    FileRow { row: Vec<u8> },
    EncodedColumn { col: Vec<TestField> },
    PolynomialEvaluation { evaluation_result: Vec<TestField> },
    ReshapeEvaluation { expected_result: TestField, original_result_vector: Vec<TestField>, original_columns: Vec<PoSColumn>, new_result_vector: Vec<TestField>, new_columns: Vec<PoSColumn> },
    EditOrAppendEvaluation { expected_result: TestField, original_result_vector: Vec<TestField>, original_columns: Vec<PoSColumn>, new_result_vector: Vec<TestField>, new_columns: Vec<PoSColumn> },
    ServerKeepAlive,
    FileDeleted { filename: String },
    BadResponse { error: String },
}