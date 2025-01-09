use blake3::Hasher as Blake3;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use proof_of_storage::fields;
use proof_of_storage::fields::{is_power_of_two, WriteableFt63};
use proof_of_storage::lcpc_online::{
    convert_file_data_to_commit, CommitDimensions, CommitRequestType,
};

fn file_to_commit_root(c: &mut Criterion) {
    // let total_size: Vec<u64> = (3..=6).map(|order| pow(10, order)).collect();
    let files = std::fs::read_dir("test_files/").unwrap();
    let mut group = c.benchmark_group("make_commit");
    for file in files {
        let file_path = file.unwrap().path();

        let encoded_file_data = fields::random_writeable_field_vec::<WriteableFt63>(logsize);
        let size = 1 << logsize;
        group.throughput(Throughput::Elements(size));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter(|| {
                let _commit = convert_file_data_to_commit::<Blake3, _>(
                    &encoded_file_data,
                    CommitRequestType::Commit,
                    CommitDimensions::Square,
                );
            })
        });
    }
}

fn get_default_dims(len: usize) -> (usize, usize) {
    let data_min_width = (len as f32).sqrt().ceil() as usize;
    let num_pre_encoded_columns = if is_power_of_two(data_min_width) {
        data_min_width
    } else {
        data_min_width.next_power_of_two()
    };
    let num_encoded_matrix_columns = (num_pre_encoded_columns + 1).next_power_of_two();
    (num_pre_encoded_columns, num_encoded_matrix_columns)
}

criterion_group!(benches, file_to_commit_root);
criterion_main!(benches);
