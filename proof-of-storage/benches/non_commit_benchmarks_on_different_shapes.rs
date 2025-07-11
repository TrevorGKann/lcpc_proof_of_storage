#![allow(unused)]
use crate::flamegraph_profiler::FlamegraphProfiler;
use anyhow::{bail, ensure, Result};
use blake3::traits::digest::Output;
use blake3::Hasher as Blake3;
use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use ff::Field;
use itertools::Itertools;
use lcpc_2d::LcEncoding;
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::data_field::DataField;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::{
    _encode_row, _get_POS_soundness_n_cols, client_online_verify_column_leaves,
    client_online_verify_column_paths, form_side_vectors_for_polynomial_evaluation_from_point,
    get_PoS_soudness_n_cols,
};
use proof_of_storage::networking::client::{get_column_indicies_from_random_seed, request_proof};
use proof_of_storage::{fields, lcpc_online};
use rand::Rng;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::{env, fs};
use ulid::Ulid;

mod bench_utils;
mod flamegraph_profiler;

type BenchField = WriteableFt63;
type BenchDigest = Blake3;
type BenchFileHandler = FileHandler<BenchDigest, BenchField, LigeroEncoding<BenchField>>;
const EDIT_BYTES: usize = 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PremadeFiles {
    ulid: Ulid,
    original_file: PathBuf,
    pre_encoded_size: usize,
    encoded_size: usize,
}

// const save_state_location: PathBuf = PathBuf.from();

fn non_commit_benchmarks_on_different_shapes_main(c: &mut Criterion) {
    bench_utils::start_bench_subscriber();

    let test_ulid = Ulid::new();
    // let test_file_path = PathBuf::from("test_files/10000000000_byte_file.bytes");
    let test_file_path = PathBuf::from("test_files/100000_byte_file.bytes");
    let total_file_bytes = std::fs::metadata(&test_file_path).unwrap().len();
    let total_field_elements = (total_file_bytes as u32 / BenchField::BYTE_CAPACITY as u32);

    let mut previous_run = load_previous_run().unwrap();

    let square_ratio = total_field_elements.isqrt().ilog2();
    let wide_ratio = (total_field_elements / (total_field_elements.ilog2())).ilog2();
    // let powers_of_two_for_pre_encoded_columns: Vec<u32> = (16..22).collect();
    let powers_of_two_for_pre_encoded_columns = [square_ratio];

    let mut group = c.benchmark_group("edit_file");
    group.throughput(Throughput::Bytes(total_file_bytes));
    group.sample_size(10);
    let bench_start_time = std::time::Instant::now();

    for power_of_two in powers_of_two_for_pre_encoded_columns {
        let pre_encoded_len = 2usize.pow(power_of_two);
        let encoded_len = 2usize.pow(power_of_two + 1);
        tracing::info!(
            "Working with {} pre-encoded columns and {} encoded columns",
            pre_encoded_len,
            encoded_len
        );

        let file_handler = open_file_handler_optimistic_from_prior(
            &test_ulid,
            &test_file_path,
            &mut previous_run,
            pre_encoded_len,
            encoded_len,
        );

        let start_time = std::time::Instant::now();

        onetime_column_bytes(file_handler.clone());

        edit_benchmark(
            &mut group,
            pre_encoded_len,
            encoded_len,
            file_handler.clone(),
        );

        client_edit_benchmark(&mut group, file_handler.clone());

        retrieve_column_benchmark(&mut group, file_handler.clone());

        verify_column_benchmark(&mut group, file_handler.clone());

        server_read_benchmark(&mut group, file_handler.clone());

        client_read_benchmark(&mut group, file_handler.clone());

        // debug: delete prior run for space
        tracing::info!("Deleting previous run");
        let _guard = file_handler.lock().unwrap();
        fs::remove_file(_guard.get_raw_file_handle()).unwrap();
        fs::remove_file(_guard.get_merkle_file_handle()).unwrap();
        fs::remove_file(_guard.get_encoded_file_handle()).unwrap();

        tracing::info!(
            "that round of testing took {}m",
            start_time.elapsed().as_secs() / 60
        );
    }
    tracing::info!(
        "Total test time: {}h",
        bench_start_time.elapsed().as_secs() / 3600
    );
}

fn open_file_handler_optimistic_from_prior(
    test_ulid: &Ulid,
    test_file_path: &PathBuf,
    previous_run: &mut Vec<PremadeFiles>,
    pre_encoded_len: usize,
    encoded_len: usize,
) -> Arc<Mutex<BenchFileHandler>> {
    let index = previous_run.iter().position(|premade| {
        premade.original_file.to_str() == test_file_path.to_str()
            && premade.pre_encoded_size == pre_encoded_len
            && premade.encoded_size == encoded_len
    });
    let file_handler: Option<Arc<Mutex<BenchFileHandler>>> = if index.is_some() {
        let premade_file = previous_run[index.unwrap()].clone();
        let file_handler = BenchFileHandler::new_attach_to_existing_ulid(
            &env::current_dir()
                .unwrap()
                .join(constants::SERVER_FILE_FOLDER),
            &premade_file.ulid,
        );
        if file_handler.is_ok() {
            tracing::debug!("save state found, extracing old save data");
            Some(Arc::new(Mutex::new(file_handler.unwrap())))
        } else {
            tracing::warn!("bad save state encountered, purging and re-encoding");
            previous_run.remove(index.unwrap());
            write_state(previous_run).unwrap();
            None
        }
    } else {
        None
    };
    // For some reason `unwrap_or` is always going into the or state
    if let Some(file_handler) = file_handler {
        file_handler
    } else {
        // default case, if no save state is found **or** if save state is corrupt
        tracing::warn!("No save state found, doing initial encoding of file");

        let test_dir = bench_utils::get_bench_single_use_subdir(&test_ulid.to_string());
        tracing::debug!("test directory is {}", test_dir.display());
        let unencoded_test_file = test_dir.join(format!(
            "{}.{}",
            test_ulid.to_string(),
            constants::UNENCODED_FILE_EXTENSION
        ));
        std::fs::copy(&test_file_path, &unencoded_test_file).expect("couldn't copy test file");

        let start_time = std::time::Instant::now();
        let file_handler = {
            let file_handler = BenchFileHandler::create_from_unencoded_file(
                test_ulid,
                Some(&unencoded_test_file),
                pre_encoded_len,
                encoded_len,
            )
            .expect("couldn't open the encoded file");
            Arc::new(Mutex::new(file_handler))
        };
        tracing::info!(
            "Done encoding file, that took {}ms",
            start_time.elapsed().as_millis()
        );
        append_to_previous_run(test_ulid, test_file_path, pre_encoded_len, encoded_len).unwrap();
        file_handler
    }
}

fn load_previous_run() -> Result<Vec<PremadeFiles>> {
    let save_file = PathBuf::from("edit_bench_saved_file.bench");
    if !save_file.exists() {
        File::create(save_file)?;
        tracing::warn!("no previous save detected");
        return Ok(vec![]);
    }

    // deserialize save_file
    let data = serde_json::from_str(&std::fs::read_to_string(save_file)?)?;

    Ok(data)
}

fn append_to_previous_run(
    ulid: &Ulid,
    original_file: &PathBuf,
    pre_encoded_size: usize,
    encoded_size: usize,
) -> Result<()> {
    let save_file = PathBuf::from("edit_bench_saved_file.bench");
    let mut data: Vec<PremadeFiles> = if save_file.exists() && save_file.metadata()?.len() > 0 {
        // deserialize save_file
        serde_json::from_str(&std::fs::read_to_string(&save_file)?)?
    } else {
        File::create(&save_file)?;
        Vec::new()
    };

    let new_premade = PremadeFiles {
        ulid: ulid.clone(),
        original_file: original_file.clone(),
        pre_encoded_size,
        encoded_size,
    };
    data.push(new_premade);

    write_state(&data)?;

    Ok(())
}
fn write_state(state: &Vec<PremadeFiles>) -> Result<()> {
    let save_file = PathBuf::from("edit_bench_saved_file.bench");
    if !save_file.exists() {
        File::create(&save_file)?;
    }
    std::fs::write(
        &save_file,
        serde_json::to_string_pretty::<Vec<PremadeFiles>>(&state)?,
    )?;
    Ok(())
}

fn edit_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    group.bench_with_input(
        BenchmarkId::new("editing file with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.iter_batched(
                || {
                    let file_handler_lock =
                        file_handler.lock().expect("couldn't lock file handler");
                    let total_file_bytes = file_handler_lock.get_total_data_bytes();

                    let mut rand = ChaChaRng::from_entropy();
                    let mut random_bytes_to_write = [0u8; EDIT_BYTES];
                    rand.fill_bytes(&mut random_bytes_to_write);
                    let start_index =
                        rand.gen_range(0..total_file_bytes - random_bytes_to_write.len());
                    drop(file_handler_lock);
                    (random_bytes_to_write, start_index)
                },
                |input| {
                    let (random_bytes_to_write, start_index) = input;
                    let mut encoded_file_reader_lock =
                        file_handler.lock().expect("couldn't lock file handler");
                    let _original_bytes = encoded_file_reader_lock
                        .edit_bytes(start_index, &random_bytes_to_write)
                        .expect("couldn't edit bytes in the file");
                    std::mem::drop(encoded_file_reader_lock);
                },
                BatchSize::LargeInput,
            )
        },
    );
}

fn client_edit_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let (pre_encoded_len, encoded_len, rows) = {
        let mut file_handler_lock = file_handler.lock().expect("couldn't lock file handler");
        file_handler_lock
            .get_dimensions()
            .expect("couldn't get file dimensions")
    };

    group.bench_with_input(
        BenchmarkId::new("verifying edited file with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.iter_batched(
                || {
                    let mut file_handler_lock = file_handler.lock().expect("coulnd't lock file handler");
                    let total_file_bytes = file_handler_lock.get_total_data_bytes();

                    let mut rand = ChaChaRng::from_entropy();

                    // set up everything the client would have sent
                    let mut requested_columns = get_column_indicies_from_random_seed(
                        rand.next_u64(),
                        _get_POS_soundness_n_cols(pre_encoded_len, encoded_len),
                        encoded_len,
                    );
                    let evaluation_point = BenchField::random(&mut rand);
                    let (evaluation_left_vector, evaluation_right_vector) =
                        form_side_vectors_for_polynomial_evaluation_from_point(
                            &evaluation_point,
                            rows,
                            pre_encoded_len,
                        );

                    let mut random_bytes_to_write = [0u8; EDIT_BYTES];
                    rand.fill_bytes(&mut random_bytes_to_write);
                    let start_index =
                        rand.gen_range(0..total_file_bytes as usize - random_bytes_to_write.len());
                    let start_row =
                        start_index / (BenchField::DATA_BYTE_CAPACITY as usize * pre_encoded_len);
                    let end_row = (start_index + EDIT_BYTES)
                        / (BenchField::DATA_BYTE_CAPACITY as usize * pre_encoded_len);

                    // get the challenge results from pre-edit
                    let pre_edit_rows = (start_row..=end_row)
                        .into_iter()
                        .map(|row_index| file_handler_lock.get_unencoded_row(row_index))
                        .collect::<Result<Vec<Vec<u8>>>>()
                        .expect("could not get unencoded rows of pre-edited file")
                        .into_iter().flatten().collect::<Vec<_>>();

                    let pre_edit_columns = file_handler_lock
                        .read_full_columns(ColumnsToCareAbout::Only(requested_columns.clone()))
                        .expect("couldn't get pre-edit columns");
                    let pre_edit_polynomial_result = file_handler_lock
                        .left_multiply_unencoded_matrix_by_vector(&evaluation_left_vector)
                        .expect("couldn't evaluate pre-edit polynomial");
                    let pre_edit_root = file_handler_lock.get_commit_root().expect("couldn't get commit root pre-edit");

                    // make the edit and get the challenge
                    file_handler_lock
                        .edit_bytes(start_index, &random_bytes_to_write)
                        .expect("couldn't edit the bytes for client verification");

                    let post_edit_columns = file_handler_lock
                        .read_full_columns(ColumnsToCareAbout::Only(requested_columns.clone()))
                        .expect("couldn't get post-edit columns");
                    let post_edit_polynomial_result = file_handler_lock
                        .left_multiply_unencoded_matrix_by_vector(&evaluation_left_vector)
                        .expect("couldn't evaluate post-edit polynomial");
                    let post_edit_root = file_handler_lock.get_commit_root().expect("couldn't get commit root post-edit");

                    (
                        random_bytes_to_write,
                        start_index,
                        start_row,
                        end_row,
                        evaluation_point,
                        requested_columns,
                        pre_edit_rows,
                        pre_edit_columns,
                        pre_edit_polynomial_result,
                        pre_edit_root,
                        post_edit_columns,
                        post_edit_polynomial_result,
                        post_edit_root,
                    )
                },
                |input| {
                    let (
                        random_bytes_to_write,
                        start_index,
                        start_row,
                        end_row,
                        evaluation_point,
                        requested_columns,
                        pre_edit_rows,
                        pre_edit_columns,
                        pre_edit_polynomial_result,
                        pre_edit_root,
                        post_edit_columns,
                        post_edit_polynomial_result,
                        post_edit_root,
                    ) = input;

                    let encoding: lcpc_ligero_pc::LigeroEncodingRho<BenchField, _, _> = LigeroEncoding::new_from_dims(pre_encoded_len, encoded_len);

                    // calculate the expected difference in polynomials
                    let internal_start_byte = start_index % (BenchField::DATA_BYTE_CAPACITY as usize * pre_encoded_len);
                    let mut post_edit_rows = pre_edit_rows.clone();
                    post_edit_rows[internal_start_byte..(internal_start_byte + EDIT_BYTES)].copy_from_slice(&random_bytes_to_write);

                    let pre_edit_rows = BenchField::from_byte_vec(&pre_edit_rows);
                    let post_edit_rows = BenchField::from_byte_vec(&post_edit_rows);
                    let expected_difference = fields::evaluate_field_polynomial_at_point_with_elevated_degree(
                        &post_edit_rows,
                        &evaluation_point,
                        (start_row * pre_encoded_len) as u64,
                    ) - fields::evaluate_field_polynomial_at_point_with_elevated_degree(
                        &pre_edit_rows,
                        &evaluation_point,
                        (start_row * pre_encoded_len) as u64,
                    );

                    let pre_edit_encoded_rows = pre_edit_rows
                        .chunks(pre_encoded_len)
                        .map(|chunk| {
                            let mut row = vec![BenchField::ZERO; encoded_len];
                            row[..pre_encoded_len].copy_from_slice(&chunk);
                            encoding.encode(&mut row).expect("could not encode pre-edit rows");
                            row
                        })
                        .collect::<Vec<Vec<BenchField>>>();


                    let post_edit_encoded_rows = post_edit_rows
                        .chunks(pre_encoded_len)
                        .map(|chunk| {
                            let mut row = vec![BenchField::ZERO; encoded_len];
                            row[..pre_encoded_len].copy_from_slice(&chunk);
                            encoding.encode(&mut row).expect("could not encode post-edit rows");
                            row
                        })
                        .collect::<Vec<Vec<BenchField>>>();


                    // finish evaluating the file polynomials
                    let (evaluation_left_vector, evaluation_right_vector) =
                        form_side_vectors_for_polynomial_evaluation_from_point(
                            &evaluation_point,
                            rows,
                            pre_encoded_len,
                        );

                    let old_results =
                        lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
                            &evaluation_point,
                            &pre_edit_polynomial_result,
                            rows,
                            encoded_len,
                            &requested_columns,
                            &pre_edit_columns,
                        ).expect("pre-edit polynomial evaluation failed");
                    let new_results =
                        lcpc_online::verify_full_polynomial_evaluation_wrapper_with_single_eval_point::<Blake3>(
                            &evaluation_point,
                            &post_edit_polynomial_result,
                            rows,
                            encoded_len,
                            &requested_columns,
                            &post_edit_columns,
                        ).expect("post edit polynomial evaluation failed");

                    assert_eq!(new_results - old_results, expected_difference);

                    // check that columns are correct
                    client_online_verify_column_paths(&pre_edit_root, &requested_columns, &pre_edit_columns).expect("pre-edit root did not validate");
                    client_online_verify_column_paths(&post_edit_root, &requested_columns, &post_edit_columns).expect("post edit root did not validate");

                    for (col_i, edited_column_index) in requested_columns.iter().enumerate() {
                        let old_requested_column = &pre_edit_columns[col_i];
                        let new_requested_column = &post_edit_columns[col_i];
                        for (row_i, true_row_index) in (start_row..=end_row).enumerate() {
                            assert_eq!(old_requested_column.col[true_row_index], pre_edit_encoded_rows[row_i][true_row_index]);
                            assert_eq!(new_requested_column.col[true_row_index], post_edit_encoded_rows[row_i][true_row_index]);
                        }
                    }
                },
                BatchSize::LargeInput,
            )
        },
    );
}
fn onetime_column_bytes(file_handler: Arc<Mutex<BenchFileHandler>>) {
    let mut file_handler_lock = file_handler.lock().unwrap();
    let (pre_encoded_len, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
    let columns = {
        let mut rand = ChaChaRng::from_entropy();
        let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
        let columns_to_fetch =
            get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);
        file_handler_lock
            .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch))
            .expect("couldn't get full columns from file")
    };
    let merkle_bytes = columns
        .iter()
        .map(|col| col.path.len() * size_of::<Output<BenchDigest>>())
        .sum::<usize>();
    let column_bytes = columns
        .iter()
        .map(|col| col.col.len() * BenchField::WRITTEN_BYTES_WIDTH as usize)
        .sum::<usize>();
    let total_bytes = column_bytes + merkle_bytes;
    let column_depth = columns.first().unwrap().col.len();
    let merkle_tree_size = pre_encoded_len * 2 - 1;
    let merkle_tree_depth = columns.first().unwrap().path.len();
    let columns_sent = columns.len();
    tracing::info!(
        "{columns_sent} columns were sent with a path-length of {merkle_tree_depth} and {column_depth} field elements.\
        The Merkle tree's total size was {merkle_tree_size}.\
        Together, the proof is {total_bytes} total bytes, {merkle_bytes} of which were from the path and {column_bytes} from the col elements.",
    );
}

fn retrieve_column_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let mut file_handler_lock = file_handler.lock().unwrap();
    let (pre_encoded_len, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();

    let mut rand = ChaChaRng::from_entropy();
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);

    group.bench_function(
        BenchmarkId::new("retrieving columns with X cols", pre_encoded_len),
        |b| {
            b.iter_batched(
                || {
                    get_column_indicies_from_random_seed(
                        rand.next_u64(),
                        num_cols_required,
                        encoded_len,
                    )
                },
                |input| {
                    let columns_to_fetch = input;
                    let _columns = file_handler_lock
                        .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch))
                        .expect("couldn't get full columns from file");
                },
                BatchSize::SmallInput,
            )
        },
    );
}

fn verify_column_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let (pre_encoded_len, encoded_len, _) = {
        let mut file_handler_lock = file_handler.lock().unwrap();
        file_handler_lock.get_dimensions().unwrap()
    };

    let (root, columns_to_fetch, columns_fetched) = {
        let mut file_handler_lock = file_handler.lock().unwrap();
        let (pre_encoded_len, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
        let mut rand = ChaChaRng::from_entropy();
        let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
        let columns_to_fetch =
            get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

        let root = file_handler_lock.get_commit_root().unwrap();

        let columns_fetched = file_handler_lock
            .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))
            .expect("couldn't get full columns from file");
        (root, columns_to_fetch, columns_fetched)
    };

    group.bench_function(
        BenchmarkId::new("verifying columns with X cols", pre_encoded_len),
        |b| {
            b.iter(|| {
                client_online_verify_column_paths(&root, &columns_to_fetch, &columns_fetched)
                    .expect("failed verification step")
            })
        },
    );
}

fn server_read_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let mut file_handler_lock = file_handler.lock().unwrap();
    let (pre_encoded_len, encoded_len, rows) = file_handler_lock.get_dimensions().unwrap();

    let mut rand = ChaChaRng::from_entropy();
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);

    group.bench_function(
        BenchmarkId::new("retrieving columns with X cols", pre_encoded_len),
        |b| {
            b.iter_batched(
                || {
                    let columns_to_fetch = get_column_indicies_from_random_seed(
                        rand.next_u64(),
                        num_cols_required,
                        encoded_len,
                    );
                    let row_to_read = rand.gen_range(0..rows);
                    (columns_to_fetch, row_to_read)
                },
                |input| {
                    let (columns_to_fetch, row_to_read) = input;
                    let _columns = file_handler_lock
                        .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch))
                        .expect("couldn't get full columns from file");
                    let _retrieved_row = file_handler_lock
                        .get_unencoded_row(row_to_read)
                        .expect("Couldn't get unencoded row");
                },
                BatchSize::SmallInput,
            )
        },
    );
}
fn client_read_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let mut file_handler_lock = file_handler.lock().unwrap();
    let (pre_encoded_len, encoded_len, rows) = file_handler_lock.get_dimensions().unwrap();
    let root = file_handler_lock.get_commit_root().unwrap();

    let mut rand = ChaChaRng::from_entropy();
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);

    group.bench_function(
        BenchmarkId::new("retrieving columns with X cols", pre_encoded_len),
        |b| {
            b.iter_batched(
                || {
                    let columns_to_fetch = get_column_indicies_from_random_seed(
                        rand.next_u64(),
                        num_cols_required,
                        encoded_len,
                    );
                    let row_to_read = rand.gen_range(0..rows);
                    let columns = file_handler_lock
                        .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))
                        .expect("couldn't get full columns from file");
                    let retrieved_row = file_handler_lock
                        .get_unencoded_row(row_to_read)
                        .expect("Couldn't get unencoded row");
                    (columns_to_fetch, columns, row_to_read, retrieved_row)
                },
                |input| {
                    let (columns_to_fetch, columns, row_to_read, retrieved_row) = input;

                    let encoding: lcpc_ligero_pc::LigeroEncodingRho<BenchField, _, _> =
                        LigeroEncoding::new_from_dims(pre_encoded_len, encoded_len);
                    let mut encoded_row = vec![BenchField::ZERO; encoded_len];
                    encoded_row[..pre_encoded_len]
                        .copy_from_slice(&BenchField::from_byte_vec(&retrieved_row));
                    encoding.encode(&mut encoded_row).unwrap();

                    for (column, column_index) in columns.iter().zip(columns_to_fetch.iter()) {
                        assert_eq!(column.col[row_to_read], encoded_row[*column_index]);
                    }

                    client_online_verify_column_paths(&root, &columns_to_fetch, &columns)
                        .expect("failed to verify column paths to root on verified read");
                },
                BatchSize::SmallInput,
            )
        },
    );
}
/* fn verify_leaves_benchmark(
    rt: &mut Runtime,
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let (root, columns_to_fetch, columns_fetched) = rt.block_on( {
        let mut file_handler_lock = file_handler.lock();
        let (_, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
        let mut rand = ChaChaRng::from_entropy();
        let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
        let columns_to_fetch =
            get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

        let root = file_handler_lock.get_commit_root().unwrap();

        let columns_fetched = file_handler_lock
            .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))

            .expect("couldn't get full columns from file");
        (root, columns_to_fetch, columns_fetched)
    });
    group.bench_with_input(
        BenchmarkId::new("verifying leaves with X cols", pre_encoded_len),
        &columns_fetched,
        |b, columns_fetched| {
            b.to_(&*rt).iter(||  {
                client_online_verify_column_leaves(
                    locally_derived_column_leaves,
                    requested_columns,
                    &received_columns_leaves,
                )?;
                client_online_verify_column_paths(
                    commitment_root,
                    requested_columns,
                    received_columns,
                )?;
                client_online_verify_column_paths(&root, &columns_to_fetch, columns_fetched)
            })
        },
    );
}
*/

fn custom_criterion() -> Criterion {
    Criterion::default().with_profiler(FlamegraphProfiler::new(1000))
}

criterion_group! {
name = benches;
config = custom_criterion();
targets = non_commit_benchmarks_on_different_shapes_main}
criterion_main!(benches);
