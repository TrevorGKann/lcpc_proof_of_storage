use std::hash::Hash;

use blake3::Hasher as Blake3;
use blake3::traits::digest::{Digest, Output};

use lcpc_2d::{LcEncoding, open_column, VerifierError, VerifierResult, verify_column_path};

use crate::{PoSColumn, PoSCommit, PoSEncoding, PoSField, PoSRoot};
use crate::file_metadata::ClientOwnedFileMetadata;

pub type FldT<E> = <E as LcEncoding>::F;
pub type ErrT<E> = <E as LcEncoding>::Err;


/// retreive a set of columns from a single commitment to send to a remote client for verification
pub fn server_retreive_columns<D, E>(
    comm: &PoSCommit,
    requested_columns: &[usize],
) -> Vec<PoSColumn>
    where
        D: Digest,
        E: LcEncoding,
{
    // extract the columns to open
    requested_columns
        .iter()
        .map(|&col| open_column(comm, col).unwrap())
        .collect::<Vec<PoSColumn>>()
}

/// Given a set of columns from a root hash commitment, verify that the supplied columns are valid Merkle paths to data.
pub fn client_online_verify_column_paths(
    commitment_root: &PoSRoot,
    requested_columns: &[usize],
    received_columns: &[PoSColumn],
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if received_columns.len() != requested_columns.len() {
        return Err(VerifierError::ColumnEval);
    }

    for (col_num, column) in requested_columns.iter().zip(received_columns.iter()) {
        let path = verify_column_path(column, *col_num, &commitment_root.root);
        if !path {
            return Err(VerifierError::ColumnEval);
        }
    }
    Ok(())
}


/// Given a set of columns on an initial commitment, verify that the column values match the local encoding of the file
pub fn client_online_verify_column_leaves(
    // optimization: this could either be a random linear combination or just a hash of it
    locally_derived_column_leaves: &[Output<Blake3>],
    requested_columns: &[usize],
    eceived_column_leaves: &[Output<Blake3>],
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if locally_derived_column_leaves.len() != requested_columns.len() || eceived_column_leaves.len() != requested_columns.len() {
        return Err(VerifierError::NumColOpens);
    }

    let leaves_ok = locally_derived_column_leaves.iter()
        .zip(eceived_column_leaves.iter())// double iterator through locally derived column leaves and server column leaves
        .all(|(client_leaf, server_leaf)| client_leaf == server_leaf);
    //check that all of them agree

    if !leaves_ok {
        return Err(VerifierError::NumColOpens); //todo should have the right errors here but this will have to do for now
    }

    Ok(())
}

pub fn get_PoS_soudness_n_cols(
    file_metadata: &ClientOwnedFileMetadata,
) -> usize {
    let denominator: f64 = ((1f64 + (file_metadata.columns as f64 / file_metadata.encoded_columns as f64)) / 2f64).log2();
    (-128f64 / denominator).ceil() as usize
}

pub fn client_verify_commitment(
    commitment_root: &PoSRoot,
    locally_derived_column_leaves: &[Output<Blake3>],
    requested_columns: &[usize],
    received_columns: &[PoSColumn],
    required_columns_for_soundness: usize,
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if required_columns_for_soundness < locally_derived_column_leaves.len()
        || required_columns_for_soundness < requested_columns.len()
        || required_columns_for_soundness < received_columns.len()
    {
        return Err(VerifierError::NumColOpens);
    }

    // create a vec of just the received leaves
    let received_columns_leaves: Vec<Output<Blake3>> = received_columns.iter()
        .map(|column| column.path[0])
        .collect();

    client_online_verify_column_leaves(locally_derived_column_leaves, requested_columns, &received_columns_leaves)?;
    client_online_verify_column_paths(commitment_root, requested_columns, received_columns)?;

    Ok(())
}

pub fn client_verify_polynomial_evaluation(
    commitment_root: &PoSRoot,
    left_evaluation_column: &[PoSField],
    right_evaluation_column: &[PoSField],
    evaluation_result: &PoSField,
    requested_columns: &[usize],
    received_columns: &[PoSColumn],
    required_columns_for_soundness: usize,
) -> VerifierResult<(), ErrT<PoSEncoding>> {
    todo!();
}