extern crate core;

use blake3::Hasher as Blake3;

use lcpc_2d::{LcColumn, LcCommit, LcEncoding, LcEvalProof, LcRoot};
use lcpc_ligero_pc::LigeroEncoding;

use crate::fields::writable_ft63::WriteableFt63;

pub mod fields;
pub mod networking;
mod tests;


pub type PoSField = WriteableFt63;
pub type PoSEncoding = LigeroEncoding<PoSField>;
pub type PoSCommit = LcCommit<Blake3, PoSEncoding>;
pub type PoSRoot = LcRoot<Blake3, LigeroEncoding<PoSField>>;
pub type PoSProof = LcEvalProof<Blake3, PoSEncoding>;
pub type PoSColumn = LcColumn<Blake3, PoSEncoding>;
