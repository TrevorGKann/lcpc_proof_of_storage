use crate::flamegraph_profiler::FlamegraphProfiler;
use blake3::Hasher as Blake3;
use criterion::async_executor::AsyncExecutor;
use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::{Ft253_192, WriteableFt63};
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use std::ops::Add;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use ulid::Ulid;

mod bench_utils;
mod flamegraph_profiler;

type BenchField = Ft253_192;
type BenchDigest = Blake3;
type BenchFileHandler = FileHandler<BenchDigest, BenchField, LigeroEncoding<BenchField>>;

// const save_state_location: PathBuf = PathBuf.from();

fn commit_different_shape_benchmark_main(c: &mut Criterion) {
    bench_utils::start_bench_subscriber();

    // let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    let test_file_path = PathBuf::from("test_files/10000_byte_file.bytes");
    let total_file_bytes = std::fs::metadata(&test_file_path).unwrap().len();

    let powers_of_two_for_pre_encoded_columns: Vec<u32> = (13..24).collect();

    let mut group = c.benchmark_group("commit_to_file");
    group.throughput(Throughput::Bytes(total_file_bytes));

    for power_of_two in powers_of_two_for_pre_encoded_columns {
        let pre_encoded_len = 2usize.pow(power_of_two);
        let encoded_len = 2usize.pow(power_of_two + 1);
        commit_benchmark(&mut group, pre_encoded_len, encoded_len, &test_file_path)
    }
}

fn commit_benchmark(
    group: &mut BenchmarkGroup<WallTime>,
    pre_encoded_len: usize,
    encoded_len: usize,
    source_file: &PathBuf,
) {
    tracing::info!("encoding with rate {pre_encoded_len}/{encoded_len}");
    group.bench_function(
        BenchmarkId::new("commiting to file with X cols", pre_encoded_len),
        move |b| {
            b.iter_custom(|iters| {
                let test_ulid = Ulid::new();
                let test_dir = bench_utils::get_bench_single_use_subdir(&test_ulid.to_string());
                // tracing::debug!("test directory is {}", test_dir.display());
                let unencoded_test_file = test_dir.join(format!(
                    "{}.{}",
                    test_ulid.to_string(),
                    constants::UNENCODED_FILE_EXTENSION
                ));
                std::fs::copy(&source_file, &unencoded_test_file)
                    .expect("couldn't copy test source file");
                let mut total_duration = Duration::from_secs(0);
                for _ in 0..iters {
                    // setup
                    let test_ulid = Ulid::new();
                    let test_dir = bench_utils::get_bench_single_use_subdir(&test_ulid.to_string());
                    let unencoded_test_file = test_dir.join(format!(
                        "{}.{}",
                        test_ulid.to_string(),
                        constants::UNENCODED_FILE_EXTENSION
                    ));
                    std::fs::copy(&source_file, &unencoded_test_file)
                        .expect("couldn't copy test source file");

                    // the test
                    let start = Instant::now();
                    let file_handler = BenchFileHandler::create_from_unencoded_file(
                        &test_ulid,
                        Some(&unencoded_test_file),
                        pre_encoded_len,
                        encoded_len,
                    )
                    .expect("couldn't open the encoded file");
                    total_duration = total_duration.add(start.elapsed());

                    // cleanup
                    file_handler.delete_all_files().unwrap();
                }
                total_duration
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
targets = commit_different_shape_benchmark_main}
criterion_main!(benches);
