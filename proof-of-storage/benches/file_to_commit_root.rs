use blake3::Hasher as Blake3;
use criterion::{
    criterion_group, criterion_main, AxisScale, BenchmarkId, Criterion, PlotConfiguration,
    Throughput,
};
use proof_of_storage::fields::field_generator_iter::FieldGeneratorIter;
use proof_of_storage::fields::{is_power_of_two, RandomBytesIterator, WriteableFt63};
use proof_of_storage::lcpc_online::row_generator_iter::RowGeneratorIter;
use std::io::Read;

fn stream_commit_root(c: &mut Criterion) {
    // let total_size: Vec<u64> = (3..=6).map(|order| pow(10, order)).collect();
    let files = std::fs::read_dir("test_files/").unwrap();

    let mut group = c.benchmark_group("stream_commit_root");
    let plot_option = PlotConfiguration::default().summary_scale(AxisScale::Logarithmic);
    group.plot_config(plot_option);

    for file in files {
        let file_path = file.unwrap().path();
        println!("commiting to \"{}\"", file_path.display());
        let file_size = std::fs::metadata(&file_path).unwrap().len();
        let (pre_encoded, encoded) = get_default_dims(file_size as usize);

        group.throughput(Throughput::Bytes(file_size));

        group.bench_with_input(
            BenchmarkId::new("streaming from disk", file_size),
            &file_size,
            |b, &size| {
                b.iter(|| {
                    let buf_reader =
                        std::io::BufReader::new(std::fs::File::open(&file_path).unwrap())
                            .bytes()
                            .map(|b| b.unwrap());
                    let field_iterator = FieldGeneratorIter::<_, WriteableFt63>::new(buf_reader);
                    let row_iterator =
                        RowGeneratorIter::new_ligero(field_iterator, pre_encoded, encoded);
                    let _streamed_root = row_iterator.convert_to_commit_root::<Blake3>().unwrap();
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("random bytes (no disk)", file_size),
            &file_size,
            |b, &size| {
                b.iter(|| {
                    let byte_iterator = RandomBytesIterator::new().take(file_size as usize);
                    let field_iterator = FieldGeneratorIter::<_, WriteableFt63>::new(byte_iterator);
                    let row_iterator =
                        RowGeneratorIter::new_ligero(field_iterator, pre_encoded, encoded);
                    let _streamed_root = row_iterator.convert_to_commit_root::<Blake3>().unwrap();
                })
            },
        );
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

criterion_group!(benches, stream_commit_root);
criterion_main!(benches);
