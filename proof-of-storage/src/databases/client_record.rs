use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::databases::{FileMetadata, ServerHost};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientRecord {
    pub id_string: Ulid,
    pub hosted_on: ServerHost,
    pub metadata: FileMetadata,
}