use blake3::Hasher as Blake3;
use clap::builder::PathBufValueParser;
use criterion::{
    criterion_group, criterion_main, AxisScale, BenchmarkId, Criterion, PlotConfiguration,
    Throughput,
};
use proof_of_storage::fields::field_generator_iter::FieldGeneratorIter;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::encoded_file_reader::EncodedFileReader;
use proof_of_storage::lcpc_online::encoded_file_writer::EncodedFileWriter;
use proof_of_storage::lcpc_online::file_formatter::{
    get_encoded_file_location_from_id, get_merkle_file_location_from_id,
    get_unencoded_file_location_from_id,
};
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::row_generator_iter::RowGeneratorIter;
use std::io::Read;
use std::path::PathBuf;
use rand::Rng;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
use tokio::fs::File;
use tokio::runtime::Builder;
use ulid::Ulid;


fn edit_file(c: &mut Criterion) {
    let mut rt = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .unwrap();

    let test_ulid = Ulid::new();
    let test_file_path = PathBuf::from("proof-of-storage/test_files/1_000_000_000.txt");

    let test_dir = PathBuf::from(format!(
        "proof-of-storage/bench_files/{}_test",
        test_ulid.to_string()
    ));
    std::fs::create_dir(&test_dir).unwrap();

    let mut unencoded_test_file = test_dir.clone();
    unencoded_test_file.push(get_unencoded_file_location_from_id(&test_ulid));
    let mut encoded_test_file = test_dir.clone();
    encoded_test_file.push(get_encoded_file_location_from_id(&test_ulid));
    let mut merkle_test_file = test_dir.clone();
    merkle_test_file.push(get_merkle_file_location_from_id(&test_ulid));

    let powers_of_two_for_pre_encoded_columns = (10..17).collect();
    let mut pre_encoded_len = 2.pow(powers_of_two_for_pre_encoded_columns.first().unwrap()) as usize;
    let mut encoded_len = (pre_encoded_len + 1).next_power_of_two();

    std::fs::copy(&test_file_path, &unencoded_test_file).unwrap();
    rt.block_on(async {
        let mut opened_unencoded_file = File::open(unencoded_test_file).await.unwrap();
        EncodedFileWriter::convert_unencoded_file(
            &mut opened_unencoded_file,
            encoded_test_file,
            Some(merkle_test_file),
            pre_encoded_len,
            encoded_len,
        )
        .await
        .unwrap();
    });

    let mut encoded_file_reader = rt.block_on(async {
        FileHandler::new_attach_to_existing_ulid(
            &test_dir,
            &test_ulid,
            pre_encoded_len,
            encoded_len,
        )
            .await
            .unwrap()
    });
    let total_bytes = encoded_file_reader.get_total_unencoded_bytes();
    let mut rand = ChaChaRng::from_entropy();
    let mut random_bytes_to_write = [0u8; 1024];

    let mut group = c.benchmark_group("edit_file");
    for power_of_two in powers_of_two_for_pre_encoded_columns {
        pre_encoded_len = 2.pow(power_of_two) as usize;
        encoded_len = (pre_encoded_len + 1).next_power_of_two();
        rt.block_on(async {encoded_file_reader.reshape(pre_encoded_len, encoded_len).await.unwrap()});

        group.bench_with_input(
            BenchmarkId::new("editing file with X cols", pre_encoded_len),
            &pre_encoded_len,
            |b, &pre_encoded_len| b.to_async(&rt).iter(|| async {
                rand.fill_bytes(&mut random_bytes_to_write);
                let start_byte = rand.gen_range(0..(total_bytes - random_bytes_to_write.len()));
                encoded_file_reader.edit_bytes(start_byte, &random_bytes_to_write).await.unwrap();
            }),
        );

    }
}

criterion_group!(benches, edit_file);
criterion_main!(benches);