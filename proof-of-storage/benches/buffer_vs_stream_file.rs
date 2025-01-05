use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, Throughput};
use tokio::runtime::Builder;

use proof_of_storage::fields;
use proof_of_storage::fields::WriteableFt63;

fn file_to_fields_bench(c: &mut Criterion) {
    let files = [
        "test_files/1000_byte_file.bytes".to_string(),
        "test_files/4000_byte_file.bytes".to_string(),
        "test_files/10000_byte_file.bytes".to_string(),
        "test_files/40000_byte_file.bytes".to_string(),
    ];
    let mut group = c.benchmark_group("buffer_vs_stream_file");
    for file_name in files {
        let size = std::fs::File::open(&file_name).unwrap().metadata().unwrap().len();
        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(BenchmarkId::new("buffer", size), &size,
                               |b, &size| {
                                   b.iter(|| {
                                       let mut file = std::fs::File::open(&file_name).unwrap();
                                       fields::read_file_to_field_elements_vec::<WriteableFt63>(&mut file);
                                   })
                               });

        group.bench_with_input(BenchmarkId::new("stream", size), &size,
                               |b, &size| {
                                   let rt = Builder::new_multi_thread()
                                       .worker_threads(8)
                                       .build()
                                       .unwrap();
                                   b.to_async(&rt).iter(|| async {
                                       let mut file = tokio::fs::File::open(&file_name).await
                                           .unwrap();
                                       fields::stream_file_to_field_elements_vec::<WriteableFt63>
                                           (&mut file).await;
                                   })
                               });

        group.bench_with_input(BenchmarkId::new("sync stream", size), &size,
                               |b, &size| {
                                   b.iter(|| {
                                       let mut file = std::fs::File::open(&file_name).unwrap();
                                       fields::stream_file_to_field_elements_vec_sync::<WriteableFt63>(&mut file);
                                   })
                               });
    };
}


criterion_group!(benches, file_to_fields_bench);
criterion_main!(benches);