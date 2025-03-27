#![allow(unused)]

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
use proof_of_storage::lcpc_online::encoded_file_writer::EncodedFileWriter;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
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
    tracing::info!(
        "number of rayon cores by default = {}",
        rayon::current_num_threads()
    );

    let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    // let test_file_path = PathBuf::from("test_files/10000_byte_file.bytes");
    let total_file_bytes = std::fs::metadata(&test_file_path)
        .expect("couldn't get test file's metadata")
        .len();

    // let powers_of_two_for_pre_encoded_columns: Vec<u32> = (16..20).collect();
    let powers_of_two_for_pre_encoded_columns: Vec<u32> = (16..=16).collect();

    let mut group = c.benchmark_group("commit_to_file");
    group.throughput(Throughput::Bytes(total_file_bytes));
    group.sample_size(10);

    for power_of_two in powers_of_two_for_pre_encoded_columns {
        let pre_encoded_len = 2usize.pow(power_of_two);
        let encoded_len = 2usize.pow(power_of_two + 1);
        commit_benchmark(&mut group, pre_encoded_len, encoded_len, &test_file_path);
        // let bench_directory = PathBuf::from("bench_files".to_string());
        // std::fs::remove_dir_all(&bench_directory).expect("couldn't remove bench_files folder");
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
            let test_ulid = Ulid::new();
            b.iter_custom(|iters| {
                let mut total_duration = Duration::from_secs(0);
                // setup
                let test_ulid = Ulid::new();
                let test_dir = bench_utils::get_bench_single_use_subdir(&test_ulid.to_string());
                let unencoded_test_file = test_dir.join(format!(
                    "{}.{}",
                    test_ulid.to_string(),
                    constants::UNENCODED_FILE_EXTENSION
                ));
                let encoded_test_file = test_dir.join(format!(
                    "{}.{}",
                    test_ulid.to_string(),
                    constants::ENCODED_FILE_EXTENSION
                ));
                let digest_test_file = test_dir.join(format!(
                    "{}.{}",
                    test_ulid.to_string(),
                    constants::MERKLE_FILE_EXTENSION,
                ));
                let mut source_file_handle = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(source_file)
                    .expect("Unable to open source file");
                let mut digest_file_handle = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .truncate(true)
                    .create(true)
                    .open(digest_test_file)
                    .expect("couldn't open digest file");

                // iterations
                for _ in 0..iters {
                    // std::fs::copy(&source_file, &unencoded_test_file)
                    //     .expect("couldn't copy test source file");

                    // the test
                    let start = Instant::now();
                    let file_handler = EncodedFileWriter::<
                        BenchField,
                        BenchDigest,
                        LigeroEncoding<_>,
                    >::convert_unencoded_file(
                        &mut source_file_handle,
                        &encoded_test_file,
                        Some(&mut digest_file_handle),
                        pre_encoded_len,
                        encoded_len,
                    )
                    .expect("couldn't open the encoded file");
                    // let encoded_handle = File::open(&encoded_test_file)
                    //     .expect("couldn't open encoded file after commit");
                    // encoded_handle
                    //     .sync_all()
                    //     .expect("couldn't sync encoded file");

                    total_duration = total_duration.add(start.elapsed());

                    // cleanup
                    source_file_handle
                        .seek(SeekFrom::Start(0))
                        .expect("couldn't seek to start of source file");
                    digest_file_handle
                        .seek(SeekFrom::Start(0))
                        .expect("couldn't seek to start of digest file");
                    digest_file_handle
                        .set_len(0)
                        .expect("couldn't truncate digest file to 0 len");
                    std::fs::remove_file(&encoded_test_file)
                        .expect("couldn't delete the encoded file");
                    // file_handler.delete_all_files().unwrap();
                    // std::fs::remove_dir_all(test_dir).unwrap();
                }

                std::fs::remove_dir_all(test_dir).expect("couldn't remove the test directory");
                total_duration
            })
        },
    );
}

fn custom_criterion() -> Criterion {
    Criterion::default().with_profiler(FlamegraphProfiler::new(1000))
}

criterion_group! {
name = benches;
config = custom_criterion();
targets = commit_different_shape_benchmark_main}
criterion_main!(benches);
