// #![feature(generic_const_exprs)]
#![feature(associated_type_defaults)]
#![feature(iter_array_chunks)]
#![feature(iter_collect_into)]
extern crate core;

use blake3::Hasher as Blake3;

use fields::WriteableFt63;
use lcpc_2d::{LcColumn, LcCommit, LcEvalProof, LcRoot};
use lcpc_ligero_pc::LigeroEncoding;

pub mod databases;
pub mod fields;
pub mod lcpc_online;
pub mod networking;
mod tests;

pub type PoSField = WriteableFt63;
pub type PoSEncoding = LigeroEncoding<PoSField>;
pub type PoSCommit = LcCommit<Blake3, PoSEncoding>;
pub type PoSRoot = LcRoot<Blake3, LigeroEncoding<PoSField>>;
pub type PoSProof = LcEvalProof<Blake3, PoSEncoding>;
pub type PoSColumn = LcColumn<Blake3, PoSEncoding>;
