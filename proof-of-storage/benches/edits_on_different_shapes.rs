use crate::flamegraph_profiler::FlamegraphProfiler;
use blake3::Hasher as Blake3;
use criterion::async_executor::AsyncExecutor;
use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkGroup, BenchmarkId, Criterion,
};
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::{_get_POS_soundness_n_cols, client_online_verify_column_paths};
use proof_of_storage::networking::client::get_column_indicies_from_random_seed;
use rand::Rng;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use ulid::Ulid;

mod bench_utils;
mod flamegraph_profiler;

type BenchFileHanlder = FileHandler<Blake3, WriteableFt63, LigeroEncoding<WriteableFt63>>;

fn different_shape_benchmark_main(c: &mut Criterion) {
    bench_utils::start_bench_subscriber();

    let mut rt = bench_utils::get_bench_runtime();

    let test_ulid = Ulid::new();
    let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    // et test_file_path = PathBuf::from(("test_files/10000_byte_file.bytes");

    let test_dir = bench_utils::get_bench_single_use_subdir(&test_ulid.to_string());
    println!("test directory is {}", test_dir.display());

    let unencoded_test_file = test_dir.join(format!(
        "{}.{}",
        test_ulid.to_string(),
        constants::UNENCODED_FILE_EXTENSION
    ));

    std::fs::copy(&test_file_path, &unencoded_test_file).expect("couldn't copy test file");

    let powers_of_two_for_pre_encoded_columns: Vec<u32> = (10..17).collect();

    let mut group = c.benchmark_group("edit_file");
    for power_of_two in powers_of_two_for_pre_encoded_columns {
        let pre_encoded_len = 2usize.pow(power_of_two);
        let encoded_len = 2usize.pow(power_of_two + 1);

        let start_time = std::time::Instant::now();
        println!("doing initial encoding of file");
        let mut file_handler: Arc<Mutex<BenchFileHanlder>> = rt.block_on(async {
            let file_handler = BenchFileHanlder::create_from_unencoded_file(
                &test_ulid,
                Some(&unencoded_test_file),
                pre_encoded_len,
                encoded_len,
            )
            .await
            .expect("couldn't open the encoded file");
            Arc::new(Mutex::new(file_handler))
        });
        println!(
            "Done encoding file, that took {}ms",
            start_time.elapsed().as_millis()
        );

        println!("Working with {} pre-encoded columns", pre_encoded_len);

        edit_benchmark(&mut rt, &mut group, pre_encoded_len, file_handler.clone());

        retrieve_column_benchmark(&mut rt, &mut group, pre_encoded_len, file_handler.clone());

        verify_column_benchmark(&mut rt, &mut group, pre_encoded_len, file_handler.clone());
    }
}

fn edit_benchmark(
    rt: &mut Runtime,
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHanlder>>,
) {
    group.bench_with_input(
        BenchmarkId::new("editing file with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.to_async(&*rt).iter_batched(
                || async {
                    let file_handler_lock = file_handler.lock().await;
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
                |input| async {
                    let (random_bytes_to_write, start_index, mut encoded_file_reader_lock) =
                        input.await;
                    let _original_bytes = encoded_file_reader_lock
                        .edit_bytes(start_index, &random_bytes_to_write)
                        .await
                        .expect("couldn't edit bytes in the file");
                },
                BatchSize::SmallInput,
            )
        },
    );
}

fn retrieve_column_benchmark(
    rt: &mut Runtime,
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHanlder>>,
) {
    group.bench_with_input(
        BenchmarkId::new("retrieving columns with X cols", pre_encoded_len),
        &pre_encoded_len,
        |b, &pre_encoded_len| {
            b.to_async(&*rt).iter_batched(
                || async {
                    let file_handler_lock = file_handler.lock().await;
                    let (_, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
                    let mut rand = ChaChaRng::from_entropy();
                    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
                    let columns_to_fetch = get_column_indicies_from_random_seed(
                        rand.next_u64(),
                        num_cols_required,
                        encoded_len,
                    );

                    (columns_to_fetch, file_handler_lock)
                },
                |input| async {
                    let (columns_to_fetch, mut file_handler) = input.await;
                    let _columns = file_handler
                        .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch))
                        .await
                        .expect("couldn't get full columns from file");
                },
                BatchSize::SmallInput,
            )
        },
    );
}

fn verify_column_benchmark(
    rt: &mut Runtime,
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    file_handler: Arc<Mutex<BenchFileHanlder>>,
) {
    let (root, columns_to_fetch, columns_fetched) = rt.block_on(async {
        let mut file_handler_lock = file_handler.lock().await;
        let (_, encoded_len, _) = file_handler_lock.get_dimensions().unwrap();
        let mut rand = ChaChaRng::from_entropy();
        let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
        let columns_to_fetch =
            get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

        let root = file_handler_lock.get_commit_root().unwrap();

        let columns_fetched = file_handler_lock
            .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))
            .await
            .expect("couldn't get full columns from file");
        (root, columns_to_fetch, columns_fetched)
    });

    group.bench_with_input(
        BenchmarkId::new("verifying columns with X cols", pre_encoded_len),
        &columns_fetched,
        |b, columns_fetched| {
            b.to_async(&*rt).iter(|| async {
                client_online_verify_column_paths(&root, &columns_to_fetch, columns_fetched)
            })
        },
    );
}

fn custom_criterion() -> Criterion {
    Criterion::default().with_profiler(FlamegraphProfiler::new(100))
}

criterion_group! {
name = benches;
config = custom_criterion();
targets = different_shape_benchmark_main}
criterion_main!(benches);
