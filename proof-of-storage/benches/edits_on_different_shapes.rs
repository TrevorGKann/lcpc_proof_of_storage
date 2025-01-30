use blake3::Hasher as Blake3;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::WriteableFt63;
use proof_of_storage::lcpc_online::encoded_file_writer::EncodedFileWriter;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use rand::Rng;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
use std::env;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::runtime::Builder;
use ulid::Ulid;

fn edit_file(c: &mut Criterion) {
    let mut rt = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .expect("couldn't create asynchronous runtime");

    let test_ulid = Ulid::new();
    println!(
        "current directory is {}",
        env::current_dir().unwrap().display()
    );
    let mut test_file_path = env::current_dir().unwrap();
    test_file_path.push("test_files/100000_byte_file.bytes");

    let test_dir = PathBuf::from(format!("bench_files/{}_test", test_ulid.to_string()));
    std::fs::create_dir_all(&test_dir).expect("couldn't create test directory");
    println!("all directories created");
    if !test_dir.is_dir() {
        panic!("could not create test directory??");
    }
    println!("test directory is {}", test_dir.display());

    let mut unencoded_test_file = test_dir.clone();
    unencoded_test_file.push(format!(
        "{}.{}",
        test_ulid.to_string(),
        constants::UNENCODED_FILE_EXTENSION
    ));
    let mut encoded_test_file = test_dir.clone();
    encoded_test_file.push(format!(
        "{}.{}",
        test_ulid.to_string(),
        constants::ENCODED_FILE_EXTENSION
    ));
    let mut merkle_test_file = test_dir.clone();
    merkle_test_file.push(format!(
        "{}.{}",
        test_ulid.to_string(),
        constants::MERKLE_FILE_EXTENSION
    ));

    let powers_of_two_for_pre_encoded_columns: Vec<usize> = (10..17).collect();
    let mut pre_encoded_len =
        2usize.pow(*powers_of_two_for_pre_encoded_columns.first().unwrap() as u32);
    let mut encoded_len = (pre_encoded_len + 1).next_power_of_two();

    println!(
        "Trying to copy {:?} to {:?}",
        &test_file_path, &unencoded_test_file
    );
    std::fs::copy(&test_file_path, &unencoded_test_file).expect("couldn't copy test file");
    let total_file_bytes = std::fs::metadata(&unencoded_test_file).unwrap().len();
    println!("copied file to test dir");
    rt.block_on(async {
        println!("opening (creating) merkle file at {}", merkle_test_file.display());
        let mut opened_merkle_file = OpenOptions::default().read(true).write(true).create(true).open(&merkle_test_file).await.expect("couldn't open merkle file");
        println!("opening encoded file at {}", encoded_test_file.display());
        let mut opened_unencoded_file = OpenOptions::default().read(true).write(true).open(&unencoded_test_file).await.expect("couldn't open unencoded file");

        println!("doing initial encoding of file");
        let start_time = std::time::Instant::now();
        EncodedFileWriter::<WriteableFt63, Blake3, LigeroEncoding<WriteableFt63>>::convert_unencoded_file(
            &mut opened_unencoded_file,
            &encoded_test_file,
            Some(&mut opened_merkle_file),
            pre_encoded_len,
            encoded_len,
        )
            .await
            .expect("couldn't create initial encoded file");
        println!("Done encoding file, that took {}ms", start_time.elapsed().as_millis());
    });

    let mut group = c.benchmark_group("edit_file");
    for power_of_two in powers_of_two_for_pre_encoded_columns {
        println!("Working with {} pre-encoded columns", pre_encoded_len);
        let mut file_handler = rt.block_on(async {
            let unencoded_file_path = unencoded_test_file.clone();
            let encoded_file_path = encoded_test_file.clone();
            let merkle_file_path = merkle_test_file.clone();
            FileHandler::<Blake3, WriteableFt63, LigeroEncoding<WriteableFt63>>::new_attach_to_existing_files(
                &test_ulid,
                unencoded_file_path,
                encoded_file_path,
                merkle_file_path,
                pre_encoded_len,
                encoded_len,
            )
                .await
                .expect("couldn't open the encoded file")
        });

        pre_encoded_len = 2usize.pow(power_of_two as u32);
        encoded_len = (pre_encoded_len + 1).next_power_of_two();
        println!("reshaping the file to the new spec now");
        rt.block_on(async {
            file_handler
                .reshape(pre_encoded_len, encoded_len)
                .await
                .expect(
                    format!("couldn't reshape the encoded file to {}", pre_encoded_len).as_str(),
                )
        });
        println!("file has been reshaped, starting benchmark now...");

        group.bench_with_input(
            BenchmarkId::new("editing file with X cols", pre_encoded_len),
            &pre_encoded_len,
            |b, &pre_encoded_len| {
                b.to_async(&rt).iter_batched(
                    || async {
                        let mut rand = ChaChaRng::from_entropy();
                        let mut random_bytes_to_write = [0u8; 1024];
                        rand.fill_bytes(&mut random_bytes_to_write);
                        let start_index = rand
                            .gen_range(0..total_file_bytes as usize - random_bytes_to_write.len());

                        let encoded_file_reader = FileHandler::<
                            Blake3,
                            WriteableFt63,
                            LigeroEncoding<WriteableFt63>,
                        >::new_attach_to_existing_files(
                            &test_ulid,
                            unencoded_test_file.clone(),
                            encoded_test_file.clone(),
                            merkle_test_file.clone(),
                            pre_encoded_len,
                            encoded_len,
                        )
                        .await
                        .expect("failed setup: couldn't attach to files for file reader");
                        return (random_bytes_to_write, start_index, encoded_file_reader);
                    },
                    |input| async {
                        let (random_bytes_to_write, start_index, mut encoded_file_reader) =
                            input.await;
                        println!("editing at start_byte {}", start_index);
                        encoded_file_reader
                            .edit_bytes(start_index, &random_bytes_to_write)
                            .await
                            .expect("couldn't edit bytes in the file")
                    },
                    BatchSize::SmallInput,
                )
            },
        );
    }
}
/*
(
                    || async {
                        let mut encoded_file_reader = rt.block_on(async {
                            FileHandler::<Blake3, WriteableFt63, LigeroEncoding<WriteableFt63>>::new_attach_to_existing_ulid(
                                &test_dir,
                                &test_ulid,
                                pre_encoded_len,
                                encoded_len,
                            )
                                .await
                                .unwrap()
                        });
                        let total_bytes = encoded_file_reader.get_total_unencoded_bytes();
                        encoded_file_reader
                            .reshape(pre_encoded_len, encoded_len)
                            .await
                            .unwrap();
                        let mut rand = ChaChaRng::from_entropy();
                        let mut random_bytes_to_write = [0u8; 1024];
                        (encoded_file_reader, total_bytes, rand)
                    },
                    |(mut encoded_file_reader, total_bytes, mut rand)| async {
                        rand.fill_bytes(&mut random_bytes_to_write);
                        let start_byte =
                            rand.gen_range(0..(total_bytes - random_bytes_to_write.len()));
                        encoded_file_reader
                            .edit_bytes(start_byte, &random_bytes_to_write)
                            .await
                            .unwrap();
                    },
                )
 */

criterion_group!(benches, edit_file);
criterion_main!(benches);
