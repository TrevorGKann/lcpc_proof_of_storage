use blake3::traits::digest::Digest;

use lcpc_2d::{LcEncoding, open_column, ProverError, ProverResult, verify_column_path};

use crate::{PoSColumn, PoSCommit, PoSEncoding, PoSField, PoSRoot};
use crate::file_metadata::ClientOwnedFileMetadata;

pub type FldT<E> = <E as LcEncoding>::F;
pub type ErrT<E> = <E as LcEncoding>::Err;


/// retreive a set of columns from a single commitment to send to a remote client for verification
pub fn retreive_columns<D, E>(
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
pub fn online_verify_column_paths(
    commitment_root: &PoSRoot,
    requested_columns: &[usize],
    received_columns: &[PoSColumn],
) -> ProverResult<(), ErrT<PoSEncoding>>
{
    for (col_num, column) in requested_columns.iter().zip(received_columns.iter()) {
        let path = verify_column_path(column, *col_num, &commitment_root.root);
        if !path {
            return Err(ProverError::ColumnNumber);
        }
    }
    return Ok(());
}


/// Given a set of columns on an initial commitment, verify that the column values match the local encoding of the file
pub fn online_verify_column_data(
    locally_derived_columns: &Vec<Vec<PoSField>>,
    requested_columns: &[usize],
    received_columns: &Vec<PoSColumn>,
) -> ProverResult<(), ErrT<PoSEncoding>>
{
    todo!("online_verify_column_data not yet implemented")
}

pub fn get_PoS_soudness_n_cols(
    file_metadata: &ClientOwnedFileMetadata,
) -> usize {
    let den: f64 = ((1f64 + (file_metadata.encoded_columns as f64 / file_metadata.encoded_columns as f64)) / 2f64).log2();
    (-128f64 / den).ceil() as usize
}