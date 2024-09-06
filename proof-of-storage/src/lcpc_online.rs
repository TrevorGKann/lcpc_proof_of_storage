use std::cmp::min;
use std::hash::Hash;

use anyhow::{ensure, Error, Result};
use blake3::Hasher as Blake3;
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use clap::Parser;
use ff::{Field, PrimeField};
use fffft::FieldFFT;
use futures::io::WriteAll;
use num_traits::{One, Zero};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use lcpc_2d::{FieldHash, LcColumn, LcCommit, LcEncoding, open_column, VerifierError, VerifierResult, verify_column_path};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use crate::{fields, PoSColumn, PoSCommit, PoSEncoding, PoSField, PoSRoot};
use crate::databases::FileMetadata;
use crate::fields::{is_power_of_two, random_writeable_field_vec, vector_multiply};
use crate::fields::writable_ft63::WriteableFt63;
use crate::networking::client::get_columns_from_random_seed;
use crate::networking::server::get_aspect_ratio_default_from_field_len;

pub type FldT<E> = <E as LcEncoding>::F;
pub type ErrT<E> = <E as LcEncoding>::Err;

#[derive(Debug, Serialize, Deserialize)]
pub enum CommitRequestType {
    Commit,
    Leaves(Vec<usize>),
    ColumnsWithPath(Vec<usize>),
    ColumnsWithoutPath(Vec<usize>),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CommitDimensions {
    Specified {
        num_pre_encoded_columns: usize,
        num_encoded_columns: usize,
    },
    Square,
}

impl From<FileMetadata> for CommitDimensions {
    fn from(metadata: FileMetadata) -> Self {
        CommitDimensions::Specified {
            num_pre_encoded_columns: metadata.num_columns,
            num_encoded_columns: metadata.num_encoded_columns,
        }
    }
}

pub enum CommitOrLeavesOutput<D: Digest, F: PrimeField> {
    Commit(LcCommit<D, LigeroEncoding<F>>),
    Leaves(Vec<Output<D>>),
    ColumnsWithPath(Vec<LcColumn<D, LigeroEncoding<F>>>),
    ColumnsWithoutPath(Vec<Vec<F>>),
}


#[tracing::instrument]
pub fn dims_ok(num_pre_encoded_columns: usize, num_encoded_columns: usize) -> bool {
    let enc_col_power_2 = num_encoded_columns.is_power_of_two();
    let big_enough = num_pre_encoded_columns >= 1 && num_encoded_columns >= 2;
    let rho_greater_than_1 = num_encoded_columns >= 2 * num_pre_encoded_columns;
    enc_col_power_2
}

//# refactor : one function to do all the file-to-commit conversions so it doesn't keep
// erroring because of it
#[tracing::instrument]
pub fn convert_file_data_to_commit<D, F>(
    field_data: &Vec<F>,
    what_to_extract: CommitRequestType,
    dimensions: CommitDimensions,
) -> Result<CommitOrLeavesOutput<D, F>>
where
    F: PrimeField,
    D: Digest + FixedOutputReset,
{
    let data_len = field_data.len();
    ensure!(data_len > 0, "Cannot convert empty file to commit");

    // first compute matrix dimensions
    let (num_pre_encoded_columns, num_encoded_columns) = match dimensions {
        CommitDimensions::Specified {
            num_pre_encoded_columns,
            num_encoded_columns,
        } => {
            // ensure!(num_pre_encoded_columns <= data_len,
            //     "Number of pre-encoded columns must be less than or equal to the number of rows");
            ensure!(num_pre_encoded_columns >= 1 && num_encoded_columns >= 2,
                "Number of columns and encoded columns must be greater than 0");
            ensure!(num_encoded_columns.is_power_of_two(),
                "Number of encoded columns must be a power of 2");
            ensure!(num_encoded_columns >= 2 * num_pre_encoded_columns,
                "Number of encoded columns must be greater than 2 * number of columns");

            (num_pre_encoded_columns, num_encoded_columns)
        }
        CommitDimensions::Square => {
            let data_min_width = (data_len as f32).sqrt().ceil() as usize;
            let num_pre_encoded_columns = if is_power_of_two(data_min_width) {
                data_min_width
            } else {
                data_min_width.next_power_of_two()
            };
            let num_encoded_matrix_columns = (num_pre_encoded_columns + 1).next_power_of_two();
            (num_pre_encoded_columns, num_encoded_matrix_columns)
        }
    };

    let num_matrix_rows = usize::div_ceil(data_len, num_pre_encoded_columns);

    // define encoding
    let encoding = LigeroEncoding::<F>::new_from_dims(num_pre_encoded_columns, num_encoded_columns);

    // encode the data

    return match what_to_extract {
        CommitRequestType::Commit => {
            let commit = LcCommit::<D, LigeroEncoding<F>>::commit(
                &field_data,
                &encoding,
            )?;
            Ok(CommitOrLeavesOutput::Commit(commit))
        }
        CommitRequestType::Leaves(requested_leaves) => {
            // todo: eventual optimization is not to compute the entire FFT but rather evaluate the FFT as a function at each of the requested points

            let mut coeffs_with_padding = vec![F::ZERO; num_matrix_rows * num_pre_encoded_columns];
            let mut encoded_coefs = vec![F::ZERO; num_matrix_rows * num_encoded_columns];

            // local copy of coeffs with padding
            coeffs_with_padding
                .par_chunks_mut(num_pre_encoded_columns)
                .zip(field_data.par_chunks(num_pre_encoded_columns))
                .for_each(|(base_row, row_to_copy)|
                base_row[..row_to_copy.len()].copy_from_slice(row_to_copy)
                );

            // now compute FFTs
            encoded_coefs.par_chunks_mut(num_encoded_columns)
                .zip(coeffs_with_padding.par_chunks(num_pre_encoded_columns))
                .try_for_each(|(r, c)| {
                    r[..c.len()].copy_from_slice(c);
                    encoding.encode(r)
                })?;

            // now set up the hashing digests
            let mut digests = Vec::with_capacity(requested_leaves.len());
            for _ in 0..requested_leaves.len() {
                let mut new_hasher = D::new();
                Digest::update(&mut new_hasher, <Output<D> as Default>::default());
                digests.push(new_hasher);
            }

            // for each row, update the digests from the requested columns
            for row in 0..num_matrix_rows {
                for (column_index, mut hasher) in requested_leaves.iter()
                    .zip(digests.iter_mut()) {
                    encoded_coefs[row * num_encoded_columns + column_index].digest_update(hasher);
                }
            }
            let column_digest_results = digests
                .into_iter()
                .map(|hasher|
                hasher.finalize())
                .collect();

            Ok(CommitOrLeavesOutput::Leaves(column_digest_results))
        }
        CommitRequestType::ColumnsWithoutPath(requested_columns) => {
            let mut coeffs_with_padding = vec![F::ZERO; num_matrix_rows * num_pre_encoded_columns];
            let mut encoded_coefs = vec![F::ZERO; num_matrix_rows * num_encoded_columns];

            // local copy of coeffs with padding
            coeffs_with_padding
                .par_chunks_mut(num_pre_encoded_columns)
                .zip(field_data.par_chunks(num_pre_encoded_columns))
                .for_each(|(base_row, row_to_copy)|
                base_row[..row_to_copy.len()].copy_from_slice(row_to_copy)
                );

            // now compute FFTs
            encoded_coefs.par_chunks_mut(num_encoded_columns)
                .zip(coeffs_with_padding.par_chunks(num_pre_encoded_columns))
                .try_for_each(|(r, c)| {
                    r[..c.len()].copy_from_slice(c);
                    encoding.encode(r)
                })?;

            // now extract the columns
            let mut columns = Vec::with_capacity(requested_columns.len());
            for column_index in requested_columns.iter() {
                columns.push(Vec::with_capacity(num_matrix_rows));
            }

            for row in 0..num_matrix_rows {
                for (column_index, index) in requested_columns.iter().enumerate() {
                    columns[column_index].push(encoded_coefs[row * num_encoded_columns + index]);
                }
            }

            Ok(CommitOrLeavesOutput::ColumnsWithoutPath(columns))
        }
        CommitRequestType::ColumnsWithPath(requested_columns) => {
            let commit = LigeroCommit::<D, F>::commit(
                &field_data,
                &encoding,
            )?;

            let columns = requested_columns
                .iter()
                .map(|&column_index| open_column::<D, LigeroEncoding<F>>(&commit, column_index).unwrap())
                .collect::<Vec<LcColumn<D, LigeroEncoding<F>>>>();

            Ok(CommitOrLeavesOutput::ColumnsWithPath(columns))
        }
    };
}


/// retreive a set of columns from a single commitment to send to a remote client for verification
pub fn server_retreive_columns(
    comm: &PoSCommit,
    requested_columns: &Vec<usize>,
) -> Vec<PoSColumn>
{
    // extract the columns to open
    requested_columns
        .iter()
        .map(|&column_index| open_column(comm, column_index).unwrap())
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

/// Given a set of columns from a root hash commitment, verify that the supplied columns are valid Merkle paths to data.
pub fn client_online_verify_column_paths_without_full_columns(
    commitment_root: &PoSRoot,
    requested_columns: &[usize],
    received_columns_digests: &[Output<Blake3>],
    received_column_paths: &Vec<Vec<Output<Blake3>>>,
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if received_column_paths.len() != requested_columns.len() {
        return Err(VerifierError::ColumnEval);
    }

    let mut digest = Blake3::new();

    for ((col_num, column_path), column_digest)
    in requested_columns.iter()
        .zip(received_column_paths.iter())
        .zip(received_columns_digests.iter())
    {
        // check Merkle path
        let mut hash = column_digest.to_owned();
        let mut col = col_num.to_owned();
        for p in column_path {
            if col % 2 == 0 {
                Digest::update(&mut digest, &hash);
                Digest::update(&mut digest, p);
            } else {
                Digest::update(&mut digest, p);
                Digest::update(&mut digest, &hash);
            }
            hash = digest.finalize_reset();
            col >>= 1;
        }

        let paths_match = hash == (commitment_root.root as Output<Blake3>);
        if !paths_match {
            return Err(VerifierError::ColumnEval);
        }
    }
    Ok(())
}


/// Given a set of columns on an initial commitment, verify that the column values match the local encoding of the file
pub fn client_online_verify_column_leaves(
    // optimization: this could either be a random linear combination or just a hash of it
    locally_derived_column_leaves: &Vec<Output<Blake3>>,
    requested_columns: &[usize],
    received_column_leaves: &[Output<Blake3>],
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if locally_derived_column_leaves.len() != requested_columns.len() || received_column_leaves.len() != requested_columns.len() {
        return Err(VerifierError::NumColOpens);
    }

    let leaves_ok = locally_derived_column_leaves.iter()
        .zip(received_column_leaves.iter()) // double iterator through locally derived column leaves and server column leaves
        .all(|(client_leaf, server_leaf)| client_leaf == server_leaf);
    //check that all of them agree

    for (client_leaf, server_leaf) in locally_derived_column_leaves.iter().zip(received_column_leaves.iter()).take(5) {
        tracing::trace!("client leaf: {:x}, server leaf: {:x}", client_leaf, server_leaf);
    }

    if !leaves_ok {
        return Err(VerifierError::NumColOpens); //todo should have the right errors here but this will have to do for now
    }

    Ok(())
}

pub fn get_PoS_soudness_n_cols(
    file_metadata: &FileMetadata,
) -> usize {
    let denominator: f64 = ((1f64 + (file_metadata.num_columns as f64 / file_metadata.num_encoded_columns as f64)) / 2f64).log2();
    let theoretical_min = (-128f64 / denominator).ceil() as usize;
    min(theoretical_min, file_metadata.num_encoded_columns)
}

pub fn client_verify_commitment(
    commitment_root: &PoSRoot,
    locally_derived_column_leaves: &Vec<Output<Blake3>>,
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
    let received_columns_leaves: Vec<Output<Blake3>> = received_columns
        .iter()
        .map(hash_column_to_digest::<Blake3>)
        .collect();

    client_online_verify_column_leaves(locally_derived_column_leaves, requested_columns, &received_columns_leaves)?;
    client_online_verify_column_paths(commitment_root, requested_columns, received_columns)?;

    Ok(())
}

pub fn client_verify_commitment_without_full_columns(
    commitment_root: &PoSRoot,
    locally_derived_column_leaves: &Vec<Output<Blake3>>,
    requested_columns: &[usize],
    received_column_digests: &Vec<Output<Blake3>>,
    received_column_paths: &Vec<Vec<Output<Blake3>>>,
    required_columns_for_soundness: usize,
) -> VerifierResult<(), ErrT<PoSEncoding>>
{
    if required_columns_for_soundness < locally_derived_column_leaves.len()
        || required_columns_for_soundness < requested_columns.len()
        || required_columns_for_soundness < received_column_digests.len()
    //todo need to add check that received_column_paths are all the right length
    {
        return Err(VerifierError::NumColOpens);
    }

    client_online_verify_column_leaves(locally_derived_column_leaves, requested_columns, received_column_digests)?;
    client_online_verify_column_paths_without_full_columns(commitment_root, requested_columns, received_column_digests, received_column_paths)?;

    Ok(())
}


pub fn hash_column_to_digest<D>(
    column: &PoSColumn
) -> Output<D>
where
    D: Digest,
{
    let mut hasher = D::new();
    Digest::update(&mut hasher, <Output<D> as Default>::default());
    for e in &column.col[..] {
        e.digest_update(&mut hasher);
    }

    // check Merkle path
    hasher.finalize()
}

pub fn verifiable_polynomial_evaluation(
    commitment: &PoSCommit,
    left_evaluation_column: &[PoSField],
) -> Vec<PoSField>
{
    // view commitment.coefs as a matrix with shape commitment.n_rows by commitment.n_columns
    // right multiply the commitment.coefs matrix by the right_evaluation_column vector and return
    // results as a vector.
    let mut result_vector = vec![PoSField::zero(); commitment.n_cols];

    let mut commit_as_columns: Vec<Vec<PoSField>> = Vec::with_capacity(commitment.n_cols);
    for _ in 0..commitment.n_cols {
        commit_as_columns.push(Vec::with_capacity(commitment.n_rows));
    }

    for (i, entry) in commitment.comm.iter().enumerate() {
        commit_as_columns[i % commitment.n_cols].push(*entry);
    }

    for (i, column) in commit_as_columns.iter().enumerate() {
        let result_entry = vector_multiply(left_evaluation_column, column);
        result_vector[i] = result_entry;
    }

    // for (mut result_entry, column) in result_vector
    //     .iter_mut()
    //     .zip(commit_as_columns.iter())
    // {
    //     result_entry = &mut vector_multiply(left_evaluation_column, column);
    // }
    result_vector
}

/// Note: this does not verify the columns themselves, only the evaluation of the polynomial
pub fn verify_proper_partial_polynomial_evaluation<D>(
    left_evaluation_column: &[PoSField],
    evaluation_result_vector: &[PoSField],
    requested_columns_indices: &[usize],
    received_columns: &[PoSColumn],
) -> VerifierResult<(), ErrT<PoSEncoding>>
where
    D: Digest,
{
    let result_values_that_match_column_indices: Vec<&WriteableFt63> = evaluation_result_vector
        .iter()
        .enumerate()
        .filter(|(i, _)| requested_columns_indices.contains(i))
        .map(|(_, value)| value)
        .collect();

    for (received_column, received_value) in received_columns.iter().zip(result_values_that_match_column_indices.iter()) {
        let expected_result = fields::vector_multiply(left_evaluation_column, &received_column.col[..]);
        if expected_result != **received_value {
            return Err(VerifierError::ColumnEval);
        }
    }


    // for (i, result_entry) in evaluation_result_vector
    //     .iter()
    //     .enumerate()
    //     .filter(|(i, _)| requested_columns_indices.contains(i)) {
    //     let expected_result = fields::vector_multiply(left_evaluation_column, &received_columns[i].col[..]);
    //     if expected_result != *result_entry {
    //         return Err(VerifierError::ColumnEval);
    //     }
    // }
    Ok(())
}

/// does not verify the columns themselves, only the evaluation of the polynomial
/// todo: verify columns
pub fn verifiable_full_polynomial_evaluation<D>(
    left_evaluation_column: &[WriteableFt63],
    right_evaluation_column: &[WriteableFt63],
    received_result_vector: &[WriteableFt63],
    requested_column_indices: &[usize],
    received_columns: &[PoSColumn],
) -> VerifierResult<WriteableFt63, ErrT<PoSEncoding>>
where
    D: Digest,
{
    verify_proper_partial_polynomial_evaluation::<D>(left_evaluation_column, received_result_vector, requested_column_indices, received_columns)?;

    let decoded_result_vector = decode_row(received_result_vector.to_vec())?;
    let result = fields::vector_multiply(&(decoded_result_vector as Vec<WriteableFt63>), right_evaluation_column);
    Ok(result)
}

/// Verifies the evaluation of a polynomial on a single evaluation point given a set of columns 
/// and a result vector from the full matrix
pub fn verifiable_full_polynomial_evaluation_wrapper_with_single_eval_point<D>(
    evaluation_point: &WriteableFt63,
    received_result_vector: &[WriteableFt63],
    n_rows: usize,
    n_cols: usize,
    requested_column_indices: &[usize],
    received_columns: &[PoSColumn],
) -> VerifierResult<WriteableFt63, ErrT<PoSEncoding>>
where
    D: Digest,
{
    let mut left_eval_column: Vec<WriteableFt63> = Vec::with_capacity(n_rows);
    let mut right_eval_column: Vec<WriteableFt63> = Vec::with_capacity(n_cols);
    let mut right_accumulator = WriteableFt63::one();

    // right column should be [1, x, x^2, ..., x^n-1]
    for _ in 0..n_rows {
        right_eval_column.push(right_accumulator);
        right_accumulator *= evaluation_point;
    }
    // right_accumulator will end up at x^n

    // left column, then, should be [1, x^n, x^2n, ...]
    let mut left_accumulator = WriteableFt63::one();
    for _ in 0..n_rows {
        left_eval_column.push(left_accumulator);
        left_accumulator *= right_accumulator;
    }

    verifiable_full_polynomial_evaluation::<D>(
        &left_eval_column, &right_eval_column,
        received_result_vector, requested_column_indices,
        received_columns)
}

pub fn decode_row(
    mut row: Vec<PoSField>,
    //todo need to know the expected len of the output vector to trim ending zeros
) -> Result<Vec<FldT<PoSEncoding>>, ErrT<PoSEncoding>>
{
    <PoSField as FieldFFT>::ifft_oi(<Vec<PoSField> as AsMut<[PoSField]>>::as_mut(&mut row))?;
    Ok(row)
}

#[test]
fn encode_then_decode_row() {
    // random row of coefficients
    let mut row = fields::random_writeable_field_vec(4);
    // size the encoding it s.t. the encoding is a single row matrix
    let encoding = PoSEncoding::new_from_dims(1 << 4, 1 << 8);
    // encode and commit to the random vector
    let commit = PoSCommit::commit(&row, &encoding).unwrap();
    // check that the decoding works
    assert_eq!(commit.coeffs, decode_row(commit.comm).unwrap()[..commit.coeffs.len()])
}

pub fn form_side_vectors_for_polynomial_evaluation_from_point(
    evaluation_point: &WriteableFt63,
    n_rows: usize,
    n_cols: usize,
) -> (Vec<WriteableFt63>, Vec<WriteableFt63>)
{
    let mut left_eval_column: Vec<WriteableFt63> = Vec::with_capacity(n_rows);
    let mut right_eval_column: Vec<WriteableFt63> = Vec::with_capacity(n_cols);
    let mut right_accumulator = WriteableFt63::one();

    // right column should be [1, x, x^2, ..., x^n-1]
    for _ in 0..n_rows {
        right_eval_column.push(right_accumulator);
        right_accumulator *= evaluation_point;
    }
    // right_accumulator will end up at x^n

    // left column, then, should be [1, x^n, x^2n, ...]
    let mut left_accumulator = WriteableFt63::one();
    for _ in 0..n_rows {
        left_eval_column.push(left_accumulator);
        left_accumulator *= right_accumulator;
    }

    (left_eval_column, right_eval_column)
}

#[test]
fn verify_polynomial_eval() {
    let coefs = fields::random_writeable_field_vec(10);

    //todo: ought to make customizable sizes for this
    let (data_realized_width, matrix_colums, soundness) = get_aspect_ratio_default_from_field_len(coefs.len());


    let encoding = LigeroEncoding::<WriteableFt63>::new_from_dims(data_realized_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&coefs, &encoding).unwrap();
    let root = commit.get_root();


    let rng = rand::thread_rng();
    let eval_point = WriteableFt63::random(rng);
    let mut left_eval_column: Vec<WriteableFt63> = Vec::with_capacity(commit.n_rows);
    let mut right_eval_column: Vec<WriteableFt63> = Vec::with_capacity(commit.n_cols);
    let mut right_accumulator = WriteableFt63::one();

    let (left_eval_column, right_eval_column)
        = form_side_vectors_for_polynomial_evaluation_from_point(&eval_point, commit.n_rows, commit.n_cols);

    let server_partial_evaluation = verifiable_polynomial_evaluation(&commit, &left_eval_column);

    // let columns_to_fetch_indices = get_columns_from_random_seed(1337, 4, commit.n_cols);
    let columns_to_fetch_indices = (0..commit.n_cols).collect();
    let columns = server_retreive_columns(&commit, &columns_to_fetch_indices);

    let server_full_evaluation_of_polynomial = verifiable_full_polynomial_evaluation::<Blake3>(
        &left_eval_column, &right_eval_column,
        &server_partial_evaluation,
        &columns_to_fetch_indices, &columns).unwrap();
    let local_evaluation = fields::evaluate_field_polynomial_at_point(&coefs, &eval_point);

    assert_eq!(server_full_evaluation_of_polynomial, local_evaluation);
}
