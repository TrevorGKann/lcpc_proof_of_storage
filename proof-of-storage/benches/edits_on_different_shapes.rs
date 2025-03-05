#![allow(unused)]
use crate::flamegraph_profiler::FlamegraphProfiler;
use anyhow::{bail, ensure, Result};
use blake3::traits::digest::Output;
use blake3::Hasher as Blake3;
use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use itertools::Itertools;
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::data_field::DataField;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::{
    _get_POS_soundness_n_cols, client_online_verify_column_leaves,
    client_online_verify_column_paths,
};
use proof_of_storage::networking::client::get_column_indicies_from_random_seed;
use rand::Rng;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use ulid::Ulid;

mod bench_utils;
mod flamegraph_profiler;

type BenchField = WriteableFt63;
type BenchDigest = Blake3;
type BenchFileHandler = FileHandler<BenchDigest, BenchField, LigeroEncoding<BenchField>>;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PremadeFiles {
    ulid: Ulid,
    original_file: PathBuf,
    pre_encoded_size: usize,
    encoded_size: usize,
}

// const save_state_location: PathBuf = PathBuf.from();

fn edit_different_shape_benchmark_main(c: &mut Criterion) {
    bench_utils::start_bench_subscriber();

    let test_ulid = Ulid::new();
    let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    // let test_file_path = PathBuf::from("test_files/10000_byte_file.bytes");
    let total_file_bytes = std::fs::metadata(&test_file_path).unwrap().len();

    let mut previous_run = load_previous_run().unwrap();

    let powers_of_two_for_pre_encoded_columns: Vec<u32> = (13..24).collect();

    let mut group = c.benchmark_group("edit_file");
    group.throughput(Throughput::Bytes(total_file_bytes));
    let bench_start_time = std::time::Instant::now();

    for power_of_two in powers_of_two_for_pre_encoded_columns {
        let pre_encoded_len = 2usize.pow(power_of_two);
        let encoded_len = 2usize.pow(power_of_two + 1);

        let file_handler = open_file_handler_optimistic_from_prior(
            &test_ulid,
            &test_file_path,
            &mut previous_run,
            pre_encoded_len,
            encoded_len,
        );

        let start_time = std::time::Instant::now();
        tracing::info!("Working with {} pre-encoded columns", pre_encoded_len);

        onetime_column_bytes(pre_encoded_len, file_handler.clone());

        edit_benchmark(&mut group, pre_encoded_len, file_handler.clone());

        retrieve_column_benchmark(&mut group, pre_encoded_len, file_handler.clone());

        verify_column_benchmark(&mut group, pre_encoded_len, file_handler.clone());

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
            pre_encoded_len,
            encoded_len,
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
        tracing::debug!(
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
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    group.bench_with_input(
        BenchmarkId::new("editing file with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.iter_batched(
                || {
                    let file_handler_lock = file_handler.lock().unwrap();
                    let total_file_bytes = file_handler_lock
                        .get_total_data_bytes()
                        .expect("couldn't get total data bytes");

                    let mut rand = ChaChaRng::from_entropy();
                    let mut random_bytes_to_write = [0u8; 1024];
                    rand.fill_bytes(&mut random_bytes_to_write);
                    let start_index =
                        rand.gen_range(0..total_file_bytes as usize - random_bytes_to_write.len());
                    (random_bytes_to_write, start_index, file_handler_lock)
                },
                |input| {
                    let (random_bytes_to_write, start_index, mut encoded_file_reader_lock) = input;
                    let _original_bytes = encoded_file_reader_lock
                        .edit_bytes(start_index, &random_bytes_to_write)
                        .expect("couldn't edit bytes in the file");
                },
                BatchSize::SmallInput,
            )
        },
    );
}

fn onetime_column_bytes(pre_encoded_len: usize, file_handler: Arc<Mutex<BenchFileHandler>>) {
    let columns = {
        let (columns_to_fetch, mut file_handler) =
            retrieve_column_setup(&file_handler, pre_encoded_len);
        file_handler
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
        Each column was {total_bytes} total bytes, {merkle_bytes} of which were from the path and {column_bytes} from the col elements.",
    );
}

fn retrieve_column_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    group.bench_with_input(
        BenchmarkId::new("retrieving columns with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.iter_batched(
                || retrieve_column_setup(&file_handler, pre_encoded_len),
                |input| {
                    let (columns_to_fetch, mut file_handler) = input;
                    let _columns = file_handler
                        .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch))
                        .expect("couldn't get full columns from file");
                },
                BatchSize::SmallInput,
            )
        },
    );
}

fn retrieve_column_setup(
    file_handler: &Arc<Mutex<BenchFileHandler>>,
    pre_encoded_len: usize,
) -> (Vec<usize>, MutexGuard<BenchFileHandler>) {
    let file_handler_lock = file_handler.lock().unwrap();
    let (_, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
    let mut rand = ChaChaRng::from_entropy();
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
    let columns_to_fetch =
        get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

    (columns_to_fetch, file_handler_lock)
}

fn verify_column_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHandler>>,
) {
    let (root, columns_to_fetch, columns_fetched) = {
        let mut file_handler_lock = file_handler.lock().unwrap();
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
    };

    group.bench_with_input(
        BenchmarkId::new("verifying columns with X cols", pre_encoded_len),
        &columns_fetched,
        |b, columns_fetched| {
            b.iter(|| client_online_verify_column_paths(&root, &columns_to_fetch, columns_fetched))
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
targets = edit_different_shape_benchmark_main}
criterion_main!(benches);
