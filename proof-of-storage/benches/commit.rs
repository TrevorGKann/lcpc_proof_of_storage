use blake3::Hasher as Blake3;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, Throughput};

use proof_of_storage::fields;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::{CommitDimensions,
                                    CommitRequestType,
                                    convert_file_data_to_commit};

fn commit_bench(c: &mut Criterion) {
    // let total_size: Vec<u64> = (3..=6).map(|order| pow(10, order)).collect();
    let logsizes: Vec<usize> = (3..20).step_by(2).collect();
    let mut group = c.benchmark_group("make_commit");
    for logsize in logsizes.into_iter() {
        let encoded_file_data = fields::random_writeable_field_vec::<WriteableFt63>(logsize);
        let size = 1 << logsize;
        group.throughput(Throughput::Elements(size));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size,
                               |b, &size| {
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


criterion_group!(benches, commit_bench);
criterion_main!(benches);