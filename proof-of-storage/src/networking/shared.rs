use crate::databases::FileMetadata;
use crate::fields::WriteableFt63;
use crate::{PoSColumn, PoSField};
use serde::{Deserialize, Serialize};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpStream,
};
use tokio_serde::{formats::Json, Framed};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use ulid::Ulid;

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
    NewUser {
        username: String,
        password: String,
    },
    UserLogin {
        username: String,
        password: String,
    },
    UploadNewFile {
        filename: String,
        file: Vec<u8>,
        columns: usize,
        encoded_columns: usize,
    },
    StartUploadNewFileByChunks {
        filename: String,
        columns: usize,
        encoded_columns: usize,
        total_file_size: usize,
    },
    UploadFileChunk {
        file_ulid: Ulid,
        chunk: Vec<u8>,
        last_chunk: bool,
    },
    RequestFile {
        file_metadata: FileMetadata,
    },
    RequestFileRow {
        file_metadata: FileMetadata,
        row: usize,
    },
    EditFileBytes {
        file_metadata: FileMetadata,
        start_byte: usize,
        replacement_bytes: Vec<u8>,
    },
    AppendToFile {
        file_metadata: FileMetadata,
        append_data: Vec<u8>,
    },
    RequestEncodedColumn {
        file_metadata: FileMetadata,
        row: usize,
    },
    RequestProof {
        file_metadata: FileMetadata,
        columns_to_verify: Vec<usize>,
    },
    RequestPolynomialEvaluation {
        file_metadata: FileMetadata,
        evaluation_point: TestField,
    },
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
    ReshapeResponse {
        new_file_metadata: FileMetadata,
        old_file_metadata: FileMetadata,
        accepted: bool,
    },
    // functionally equivalent of ReshapeEvaluation but with the omission of differing columns
    // to expand and obviously the context
    RequestAppendEvaluation {
        old_file_metadata: FileMetadata,
        new_file_metadata: FileMetadata,
        evaluation_point: TestField,
        columns_to_expand: Vec<usize>,
    },
    // this differs from the append request because we also need to get the un-encoded rows to
    // see the difference in coefficients, whereas the append evaluation can determine which rows
    // to send based on the metadata alone.
    RequestEditEvaluation {
        old_file_metadata: FileMetadata,
        new_file_metadata: FileMetadata,
        evaluation_point: TestField,
        columns_to_expand: Vec<usize>,
        requested_unencoded_row_range_inclusive: (usize, usize),
    },
    EditOrAppendResponse {
        new_file_metadata: FileMetadata,
        old_file_metadata: FileMetadata,
        accepted: bool,
    },
    DeleteFile {
        file_metadata: FileMetadata,
    },
    ClientKeepAlive,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessages {
    UserLoginResponse {
        success: bool,
    },
    UploadingFileChunkIdentifier {
        file_ulid: Ulid,
    },
    UploadingFileChunkResponse {
        data_ok: bool,
    },
    CompactCommit {
        file_metadata: FileMetadata,
    },
    Columns {
        columns: Vec<PoSColumn>,
    },
    File {
        file: Vec<u8>,
    },
    FileRow {
        row: Vec<u8>,
    },
    EncodedColumn {
        col: Vec<TestField>,
    },
    PolynomialEvaluation {
        evaluation_result: Vec<TestField>,
    },
    ReshapeEvaluation {
        expected_result: TestField,
        original_result_vector: Vec<TestField>,
        original_columns: Vec<PoSColumn>,
        new_result_vector: Vec<TestField>,
        new_columns: Vec<PoSColumn>,
    },
    AppendEvaluation {
        original_result_vector: Vec<TestField>,
        original_columns: Vec<PoSColumn>,
        new_result_vector: Vec<TestField>,
        new_columns: Vec<PoSColumn>,
        edited_unencoded_row: Vec<WriteableFt63>,
        // optimization: this can probably just be the raw bytes and we can let the client compile
        //  them into the correct format. That gives the benefit of A) less overhead and B) less
        //  coded dependency on the `WriteableFT63` type
    },
    EditEvaluation {
        original_result_vector: Vec<TestField>,
        original_columns: Vec<PoSColumn>,
        new_result_vector: Vec<TestField>,
        new_columns: Vec<PoSColumn>,
        original_unencoded_rows: Vec<u8>,
    },
    ServerKeepAlive,
    FileDeleted {
        filename: String,
    },
    ErrorResponse {
        error: String,
    },
}
