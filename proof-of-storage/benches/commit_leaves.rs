use blake3::Hasher as Blake3;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, Throughput};

use proof_of_storage::fields;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::{CommitDimensions,
                                    CommitRequestType,
                                    convert_file_data_to_commit};
use proof_of_storage::networking::client::get_column_indicies_from_random_seed;
use proof_of_storage::networking::server::get_aspect_ratio_default_from_field_len;

fn commit_leaves_bench(c: &mut Criterion) {
    // let total_size: Vec<u64> = (3..=6).map(|order| pow(10, order)).collect();
    let logsizes: Vec<usize> = (3..20).step_by(2).collect();
    let mut group = c.benchmark_group("make_commit_to_leaves");
    for logsize in logsizes.into_iter() {
        let encoded_file_data = fields::random_writeable_field_vec::<WriteableFt63>(logsize);
        let size = 1 << logsize;
        let (
            num_pre_encoded_columns,
            num_encoded_columns,
            soundness
        ) = get_aspect_ratio_default_from_field_len(size);

        let cols_to_verify = get_column_indicies_from_random_seed(
            1337,
            soundness,
            num_encoded_columns,
        );

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size,
                               |b, &size| {
                                   b.iter(|| {
                                       let _commit = convert_file_data_to_commit::<Blake3, _>(
                                           &encoded_file_data,
                                           CommitRequestType::Leaves(cols_to_verify.clone()),
                                           CommitDimensions::Specified {
                                               num_pre_encoded_columns,
                                               num_encoded_columns,
                                           },
                                       );
                                   })
                               });
    }
}


criterion_group!(benches, commit_leaves_bench);
criterion_main!(benches);