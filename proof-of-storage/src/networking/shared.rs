use serde::{Deserialize, Serialize};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpStream,
};
use tokio_serde::{formats::Json, Framed};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use crate::databases::FileMetadata;
use crate::fields::WriteableFt63;
use crate::{PoSColumn, PoSField};

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

impl ClientMessages {
    pub fn deepsize(&self) -> usize {
        std::mem::size_of_val(self)
            + match self {
                ClientMessages::NewUser { username, password } => username.len() + password.len(),
                ClientMessages::UserLogin { username, password } => username.len() + password.len(),
                ClientMessages::UploadNewFile {
                    filename,
                    file,
                    columns,
                    encoded_columns,
                } => size_of_val(filename) + size_of_val(file),
                ClientMessages::RequestFile { file_metadata } => size_of_val(file_metadata),
                ClientMessages::RequestFileRow { file_metadata, row } => size_of_val(file_metadata),
                ClientMessages::EditFileBytes {
                    file_metadata,
                    start_byte,
                    replacement_bytes,
                } => size_of_val(file_metadata) + size_of_val(replacement_bytes),
                ClientMessages::AppendToFile {
                    file_metadata,
                    append_data,
                } => size_of_val(file_metadata) + size_of_val(append_data),
                ClientMessages::RequestEncodedColumn { file_metadata, row } => {
                    size_of_val(file_metadata)
                }
                ClientMessages::RequestProof {
                    file_metadata,
                    columns_to_verify,
                } => size_of_val(file_metadata) + size_of_val(columns_to_verify),
                ClientMessages::RequestPolynomialEvaluation {
                    file_metadata,
                    evaluation_point,
                } => size_of_val(file_metadata) + size_of_val(&evaluation_point),
                ClientMessages::RequestFileReshape {
                    file_metadata,
                    new_pre_encoded_columns,
                    new_encoded_columns,
                } => size_of_val(file_metadata),
                ClientMessages::RequestReshapeEvaluation {
                    old_file_metadata,
                    new_file_metadata,
                    evaluation_point,
                    columns_to_expand_original,
                    columns_to_expand_new,
                } => {
                    size_of_val(old_file_metadata)
                        + size_of_val(new_file_metadata)
                        + size_of_val(&evaluation_point)
                        + size_of_val(columns_to_expand_original)
                        + size_of_val(columns_to_expand_new)
                }
                ClientMessages::ReshapeResponse {
                    new_file_metadata,
                    old_file_metadata,
                    accepted,
                } => size_of_val(new_file_metadata) + size_of_val(old_file_metadata),
                ClientMessages::RequestAppendEvaluation {
                    old_file_metadata,
                    new_file_metadata,
                    evaluation_point,
                    columns_to_expand,
                } => {
                    size_of_val(old_file_metadata)
                        + size_of_val(new_file_metadata)
                        + size_of_val(&evaluation_point)
                        + size_of_val(columns_to_expand)
                }
                ClientMessages::RequestEditEvaluation {
                    old_file_metadata,
                    new_file_metadata,
                    evaluation_point,
                    columns_to_expand,
                    requested_unencoded_row_range_inclusive,
                } => {
                    size_of_val(old_file_metadata)
                        + size_of_val(new_file_metadata)
                        + size_of_val(&evaluation_point)
                        + size_of_val(columns_to_expand)
                }
                ClientMessages::EditOrAppendResponse {
                    new_file_metadata,
                    old_file_metadata,
                    accepted,
                } => size_of_val(new_file_metadata) + size_of_val(old_file_metadata),
                ClientMessages::DeleteFile { file_metadata } => size_of_val(file_metadata),
                ClientMessages::ClientKeepAlive => 0,
            }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessages {
    UserLoginResponse {
        success: bool,
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

impl ServerMessages {
    pub fn sizes(&self) -> usize {
        std::mem::size_of_val(self)
            + match self {
                ServerMessages::UserLoginResponse { success } => std::mem::size_of_val(success),
                ServerMessages::CompactCommit { file_metadata } => {
                    std::mem::size_of_val(file_metadata)
                }
                ServerMessages::Columns { columns } => std::mem::size_of_val(columns),
                _ => {
                    // todo: finish
                    //  issue: need to recursively check size of values, such as vec<column> has internal vecs within.
                    0
                } // ServerMessages::File { .. } => {}
                  // ServerMessages::FileRow { .. } => {}
                  // ServerMessages::EncodedColumn { .. } => {}
                  // ServerMessages::PolynomialEvaluation { .. } => {}
                  // ServerMessages::ReshapeEvaluation { .. } => {}
                  // ServerMessages::AppendEvaluation { .. } => {}
                  // ServerMessages::EditEvaluation { .. } => {}
                  // ServerMessages::ServerKeepAlive => {}
                  // ServerMessages::FileDeleted { .. } => {}
                  // ServerMessages::ErrorResponse { .. } => {}
            }
    }
}
