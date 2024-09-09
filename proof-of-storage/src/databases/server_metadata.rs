use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::PoSCommit;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerFileMetadata {
    pub id_string: Ulid,
    pub filename: String,
    pub owner: String,
    pub commitment: PoSCommit,
}