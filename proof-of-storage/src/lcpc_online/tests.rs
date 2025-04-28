#[cfg(test)]
mod encoded_file_io_tests {
    use std::env;
    use std::fs::{copy, metadata, read, remove_file, File, OpenOptions};
    use std::io::Read;
    use std::path::{Path, PathBuf};

    use rand::Rng;
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    use serial_test::serial;
    use ulid::Ulid;

    use lcpc_2d::log2;

    use crate::fields::data_field::DataField;
    use crate::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
    use crate::lcpc_online::encoded_file_reader::EncodedFileReader;
    use crate::lcpc_online::encoded_file_reader::{
        get_decoded_file_size_from_rate, get_encoded_file_size_from_rate,
    };
    use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
    use crate::lcpc_online::file_handler::FileHandler;
    use crate::lcpc_online::*;
    use crate::networking::client::get_column_indicies_from_random_seed;

    type TestField = WriteableFt63;

    #[test]
    #[serial]
    fn encode_then_decode_file() {
        use crate::fields::data_field::DataField;
        // use ff::Field;
        let test_file_path = PathBuf::from("test_files/test.txt");
        let file_path_encoded = PathBuf::from("test_files/test_encoded.txt");
        let file_path_decoded = PathBuf::from("test_files/test_decoded.txt");
        let metadata_path = PathBuf::from("test_files/metadata.json");

        let original_file_len = File::open(&test_file_path)
            .unwrap()
            .metadata()
            .unwrap()
            .len() as usize;
        for pre_encoded_len in (3..6)
            .map(|x| 2usize.pow(x))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            println!(
                "testing with a coding of rate {}/{}",
                pre_encoded_len, encoded_len
            );

            // encode the file
            let (_encoded_metadata, encoded_tree) = EncodedFileWriter::<
                TestField,
                Blake3,
                LigeroEncoding<TestField>,
            >::convert_unencoded_file(
                &mut File::open(&test_file_path).expect("couldn't open test file"),
                &file_path_encoded,
                None,
                Some(&metadata_path),
                pre_encoded_len,
                encoded_len,
            )
            .expect("failed initial encoding");

            let mut metadata_file = OpenOptions::new()
                .write(true)
                .read(true)
                .create(true)
                .open(&metadata_path)
                .unwrap();
            let encoded_metadata = EncodedFileMetadata::read_from_file(&mut metadata_file).unwrap();

            // check that the results are the correct size
            assert_eq!(encoded_tree.len(), encoded_len * 2 - 1);
            let encoded_file_len = metadata(&file_path_encoded)
                .expect("couldn't open test file metadata")
                .len();

            let expected_size = original_file_len
                .div_ceil(TestField::DATA_BYTE_CAPACITY as usize)
                .div_ceil(pre_encoded_len)
                * TestField::WRITTEN_BYTES_WIDTH as usize
                * encoded_len;
            println!(
                "Encoded file len: {}; expected {}",
                encoded_file_len, expected_size
            );
            assert!(
                encoded_file_len as usize == expected_size
                    || encoded_file_len as usize == expected_size * 2,
                "expected a file of size {} to be encoded into size {}",
                original_file_len,
                expected_size
            );
            assert_eq!(
                encoded_file_len as usize,
                encoded_metadata.row_capacity
                    * encoded_metadata.encoded_size
                    * TestField::WRITTEN_BYTES_WIDTH as usize,
                "File is the wrong allocated size"
            );
            assert!(encoded_metadata.row_capacity > encoded_metadata.rows_written);
            assert_eq!(encoded_metadata.encoded_size, encoded_len);
            assert_eq!(encoded_metadata.pre_encoded_size, pre_encoded_len);
            assert_eq!(original_file_len, encoded_metadata.bytes_of_data);

            // decode the file
            let encoded_file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&file_path_encoded)
                .expect("couldn't open encoded test file");
            let mut reader =
                EncodedFileReader::<TestField, Blake3, LigeroEncoding<TestField>>::new_ligero(
                    encoded_file,
                    pre_encoded_len,
                    encoded_len,
                    encoded_metadata.rows_written,
                    encoded_metadata.row_capacity,
                );
            let mut decode_target = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&file_path_decoded)
                .expect("couldn't open decode target");
            reader
                .decode_to_target_file(&mut decode_target)
                .expect("couldn't decode encoded target");

            // check that the original file and the decoded versions are the same
            let file_path_contents = read(&test_file_path).unwrap();
            let file_path_decoded_contents = read(&file_path_decoded).unwrap();
            assert_eq!(
                file_path_contents[..],
                file_path_decoded_contents[..file_path_contents.len()]
            );
        }

        remove_file(file_path_encoded).unwrap();
        remove_file(file_path_decoded).unwrap();
    }

    #[test]
    #[serial]
    fn edit_file_is_correct() {
        let mut random = ChaCha8Rng::from_entropy();
        const RANDOM_LENGTH: usize = 1028;

        let test_file_path_origin = PathBuf::from("test_files/10000_byte_file.bytes");
        let test_file_path = PathBuf::from("test_files/edit_test.txt");
        let test_file_decode_path = PathBuf::from("test_files/test_decoded.txt");
        let test_ulid = Ulid::new();

        // copy origin to test
        copy(&test_file_path_origin, &test_file_path).unwrap();

        for pre_encoded_len in (1..6)
            .map(|x| 2usize.pow(x))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            println!(
                "testing with a coding of rate {}/{}",
                pre_encoded_len, encoded_len
            );

            // copy origin to test
            copy(&test_file_path_origin, &test_file_path).unwrap();

            // get the original data
            let mut original_file_contents = read(&test_file_path).unwrap();
            let original_file_len = File::open(&test_file_path)
                .unwrap()
                .metadata()
                .unwrap()
                .len() as usize;

            let mut file_handler = FileHandler::<
                Blake3,
                TestField,
                LigeroEncoding<TestField>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
                .unwrap();

            file_handler.verify_all_files_agree().unwrap();

            for _i in 0..100 {
                // generate random data to edit the file with
                let mut random_bytes_to_write = [0u8; RANDOM_LENGTH];
                random.fill_bytes(&mut random_bytes_to_write);
                let start_index = random.gen_range(0..original_file_len - RANDOM_LENGTH);

                // get the original bytes so we can verify the edit file is returning them correctly
                let original_bytes =
                    original_file_contents[start_index..start_index + RANDOM_LENGTH].to_vec();

                // edit the file
                let (returned_original_bytes, updated_tree) = file_handler
                    .edit_bytes(start_index, &random_bytes_to_write)
                    .unwrap();
                assert_eq!(updated_tree.len(), encoded_len * 2 - 1);
                assert_eq!(original_bytes, returned_original_bytes,
                           "returned bytes weren't correct. For reference the write address was {} and the bytes to write were {:?}", start_index, random_bytes_to_write);

                // update our running tally of what the file should be
                original_file_contents[start_index..start_index + RANDOM_LENGTH]
                    .copy_from_slice(&random_bytes_to_write);

                // check that the raw buffer is correct
                let por_raw_file = std::fs::read(file_handler.get_raw_file_handle()).unwrap();
                assert_eq!(por_raw_file.len(), original_file_contents.len());
                assert_eq!(por_raw_file, original_file_contents);

                // check that the file is correct and correctly decodes to what we expect
                file_handler.verify_all_files_agree().unwrap();
                let encoded_metadata = file_handler.get_encoded_metadata();
                let mut reader =
                    EncodedFileReader::<TestField, Blake3, LigeroEncoding<TestField>>::new_ligero(
                        File::open(file_handler.get_encoded_file_handle()).unwrap(),
                        pre_encoded_len,
                        encoded_len,
                        encoded_metadata.rows_written,
                        encoded_metadata.row_capacity,
                    );

                if _i % 10 == 0 {
                    let mut decode_target = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .truncate(true)
                        .create(true)
                        .open(&test_file_decode_path)
                        .unwrap();
                    reader.decode_to_target_file(&mut decode_target).unwrap();

                    let decoded_file_contents = read(&test_file_decode_path).unwrap();
                    assert_eq!(
                        decoded_file_contents[..original_file_contents.len()],
                        original_file_contents
                    );
                }
            }

            file_handler.verify_all_files_agree().unwrap();

            file_handler.delete_all_files().unwrap()
        }
    }
    #[test]
    #[serial]
    fn append_to_file_is_correct() {
        let mut random = ChaCha8Rng::from_entropy();
        const RANDOM_LENGTH: usize = 32;

        let test_file_path_origin = PathBuf::from("test_files/test.txt");
        let test_file_path = PathBuf::from("test_files/append_test.txt");
        let test_decode_target = PathBuf::from("test_files/append_test_decode.txt");
        let test_ulid = Ulid::new();

        // // copy origin to test
        // copy(&test_file_path_origin, &test_file_path)
        //
        //     .unwrap();

        for pre_encoded_len in (1..6)
            .map(|x| 2usize.pow(x))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            println!(
                "testing with a coding of rate {}/{}",
                pre_encoded_len, encoded_len
            );

            // copy origin to test
            copy(&test_file_path_origin, &test_file_path).unwrap();

            // get the original data
            let mut original_file_contents = read(&test_file_path).unwrap();

            let mut file_handler = FileHandler::<
                Blake3,
                TestField,
                LigeroEncoding<TestField>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
                .unwrap();

            file_handler.verify_all_files_agree().unwrap();

            for _i in 0..100 {
                // generate random data to edit the file with
                let mut random_bytes_to_write = [0u8; RANDOM_LENGTH];
                random.fill_bytes(&mut random_bytes_to_write);
                // let random_bytes_to_write = *b"appended";

                // append to the file
                let updated_tree = file_handler
                    .append_bytes(random_bytes_to_write.to_vec())
                    .unwrap();
                assert_eq!(updated_tree.len(), encoded_len * 2 - 1);

                // update our running tally of what the file should be
                original_file_contents.extend_from_slice(&random_bytes_to_write);

                let por_raw_file = std::fs::read(file_handler.get_raw_file_handle()).unwrap();
                assert_eq!(por_raw_file.len(), original_file_contents.len());
                assert_eq!(por_raw_file, original_file_contents);

                // check that the file is correct and correctly decodes to what we expect
                let encoded_metadata = file_handler.get_encoded_metadata();
                let mut reader =
                    EncodedFileReader::<TestField, Blake3, LigeroEncoding<TestField>>::new_ligero(
                        File::open(file_handler.get_encoded_file_handle()).unwrap(),
                        pre_encoded_len,
                        encoded_len,
                        encoded_metadata.rows_written,
                        encoded_metadata.row_capacity,
                    );

                if _i % 10 == 0 {
                    let mut decode_target = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .truncate(true)
                        .create(true)
                        .open(&test_decode_target)
                        .unwrap();
                    reader.decode_to_target_file(&mut decode_target).unwrap();

                    let decoded_file_contents = read(&test_decode_target).unwrap();
                    assert_eq!(
                        decoded_file_contents[..original_file_contents.len()],
                        original_file_contents,
                        "contents do not match!"
                    );
                    file_handler.verify_all_files_agree().unwrap();
                }
            }
            file_handler.delete_all_files().unwrap()
        }
    }

    #[test]
    #[serial]
    fn verify_columns_are_correct() {
        let mut random = ChaCha8Rng::from_entropy();

        println!("current dir {}", env::current_dir().unwrap().display());
        let test_file_path_origin = PathBuf::from("test_files/10000_byte_file.bytes");
        let test_file_path = PathBuf::from("test_files/columns_test.txt");
        let test_ulid = Ulid::new();
        let original_file_len = File::open(&test_file_path_origin)
            .unwrap()
            .metadata()
            .unwrap()
            .len() as usize;
        let max_columns = f64::sqrt(original_file_len as _) as usize;

        for pre_encoded_len in (1..log2(max_columns))
            // for pre_encoded_len in (5..13)
            .map(|x| 2usize.pow(x as u32))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
            let expected_column_len = original_file_len
                .div_ceil(TestField::DATA_BYTE_CAPACITY as usize)
                .div_ceil(pre_encoded_len);
            println!(
                "testing with a coding of rate {}/{}; soundness requires fetching {} columns of len {}",
                pre_encoded_len, encoded_len,
                num_cols_required, expected_column_len,
            );

            // copy origin to test
            copy(&test_file_path_origin, &test_file_path).unwrap();

            let mut file_handler = FileHandler::<
                Blake3,
                TestField,
                LigeroEncoding<TestField>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
                .unwrap();

            file_handler.verify_all_files_agree().unwrap();
            let root = file_handler.get_commit_root().unwrap();

            for _i in 0..10 {
                let columns_to_fetch = get_column_indicies_from_random_seed(
                    random.next_u64(),
                    num_cols_required,
                    encoded_len,
                );
                let columns = file_handler
                    .read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))
                    .expect("couldn't get full columns from file");
                assert_eq!(columns.len(), num_cols_required);

                let sample_column = columns.first().unwrap();
                assert_eq!(sample_column.col.len(), expected_column_len);
                if _i == 0 {
                    println!("merkle length is {}", sample_column.path.len())
                }
                assert_eq!(sample_column.path.len(), log2(encoded_len));

                client_online_verify_column_paths(&root, &columns_to_fetch, &columns)
                    .expect("failed verification step")
            }

            file_handler.delete_all_files().unwrap()
        }
        _safe_delete_file(&test_file_path);
    }

    #[test]
    #[serial]
    fn reencode_rows_are_correct() {
        println!("current dir {}", env::current_dir().unwrap().display());
        let test_file_path_origin = PathBuf::from("test_files/10000_byte_file.bytes");
        let test_file_path = PathBuf::from("test_files/reencode_test.txt");
        let copy_of_encoded_file = PathBuf::from("test_files/reencode_test_copy.txt");
        let test_ulid = Ulid::new();
        let original_file_len = File::open(&test_file_path_origin)
            .unwrap()
            .metadata()
            .unwrap()
            .len() as usize;
        let max_columns = f64::sqrt(original_file_len as _) as usize;

        for pre_encoded_len in (3..log2(max_columns))
            // for pre_encoded_len in (5..13)
            .map(|x| 2usize.pow(x as u32))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            println!(
                "testing with a coding of rate {}/{}",
                pre_encoded_len, encoded_len,
            );

            // copy origin to test
            copy(&test_file_path_origin, &test_file_path).unwrap();

            let mut file_handler = FileHandler::<
                Blake3,
                TestField,
                LigeroEncoding<TestField>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
                .unwrap();

            file_handler.verify_all_files_agree().unwrap();
            std::fs::copy(
                file_handler.get_encoded_file_handle(),
                &copy_of_encoded_file,
            )
            .unwrap();

            let expected_rows = original_file_len
                .div_ceil(TestField::DATA_BYTE_CAPACITY as usize)
                .div_ceil(pre_encoded_len);
            let sample_column = file_handler
                .read_full_columns(ColumnsToCareAbout::Only(vec![0]))
                .unwrap();
            assert_eq!(expected_rows, sample_column[0].col.len());

            for row in 0..expected_rows {
                file_handler.reencode_row(row).unwrap()
            }

            assert!(_are_two_files_same(
                &copy_of_encoded_file,
                &file_handler.get_encoded_file_handle()
            ));
            file_handler.verify_all_files_agree().unwrap();

            file_handler.reencode_unencoded_file().unwrap();
            assert!(_are_two_files_same(
                &copy_of_encoded_file,
                &file_handler.get_encoded_file_handle()
            ));
            file_handler.verify_all_files_agree().unwrap();

            file_handler.delete_all_files().unwrap();
        }
        _safe_delete_file(&test_file_path);
    }

    #[test]
    fn test_that_rate_aligns() {
        use crate::fields::data_field::DataField;
        // let mut random = ChaCha8Rng::from_entropy();
        let mut random = ChaCha8Rng::seed_from_u64(1337);
        for pre_encoded_len in (2..12)
            .map(|x| 2usize.pow(x))
            .collect::<Vec<usize>>()
            .into_iter()
        {
            let encoded_len = (pre_encoded_len + 1).next_power_of_two();
            println!(
                "testing with a coding of rate {}/{}",
                pre_encoded_len, encoded_len
            );
            for _ in 0..100 {
                let random_file_len = random.gen_range(1..10000);
                let encoded_file_size = get_encoded_file_size_from_rate::<TestField>(
                    random_file_len,
                    pre_encoded_len,
                    encoded_len,
                );
                let decoded_file_size = get_decoded_file_size_from_rate::<TestField>(
                    encoded_file_size,
                    pre_encoded_len,
                    encoded_len,
                );
                assert_eq!(
                    random_file_len
                        .next_multiple_of(TestField::DATA_BYTE_CAPACITY as usize * pre_encoded_len),
                    decoded_file_size,
                    "failed with an input file size of {}; for reference the encoded size was {}",
                    random_file_len,
                    encoded_file_size
                );
            }
        }
    }

    #[test]
    #[serial]
    fn test_metadata_is_correct() {
        let mut random = ChaCha8Rng::from_entropy();
        const RANDOM_LENGTH: usize = 64;

        let test_file_path_origin = PathBuf::from("test_files/test.txt");
        let test_file_path = PathBuf::from("test_files/edit_test.txt");
        let test_ulid = Ulid::new();
        let mut pre_encoded_len = 8;
        let mut encoded_len = 16;

        // copy origin to test
        copy(&test_file_path_origin, &test_file_path).unwrap();

        // get the original data
        let mut original_file_len = File::open(&test_file_path)
            .unwrap()
            .metadata()
            .unwrap()
            .len() as usize;

        let mut file_handler = FileHandler::<
                Blake3,
                TestField,
                LigeroEncoding<TestField>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
                .unwrap();

        file_handler.verify_all_files_agree().unwrap();

        for i in 0..500 {
            let mut original_metadata = file_handler.get_encoded_metadata();

            if i % 10 == 0 {
                // randomly get a new encoding length
                pre_encoded_len = random.gen_range(2..original_file_len);
                encoded_len = (pre_encoded_len + 1).next_power_of_two();

                file_handler.reshape(pre_encoded_len, encoded_len).unwrap();
                let new_metadata = file_handler.get_encoded_metadata();
                assert_eq!(new_metadata.pre_encoded_size, pre_encoded_len);
                assert_eq!(new_metadata.encoded_size, encoded_len);
                assert_eq!(new_metadata.bytes_of_data, original_file_len);
                assert_eq!(new_metadata.bytes_of_data, original_metadata.bytes_of_data);
                assert_eq!(new_metadata.rows_written * 2, new_metadata.row_capacity);
                assert_eq!(
                    new_metadata.rows_written,
                    original_file_len
                        .div_ceil(pre_encoded_len * TestField::DATA_BYTE_CAPACITY as usize)
                );
                original_metadata = new_metadata;
            }

            // generate random data to append to the file
            let mut random_bytes_to_write = [0u8; RANDOM_LENGTH];
            random.fill_bytes(&mut random_bytes_to_write);

            file_handler
                .append_bytes(random_bytes_to_write.to_vec())
                .unwrap();
            original_file_len += RANDOM_LENGTH;
            let new_metadata = file_handler.get_encoded_metadata();
            assert_eq!(new_metadata.bytes_of_data, original_file_len);
            assert_eq!(new_metadata.ulid, test_ulid);
            assert!(
                new_metadata.rows_written <= new_metadata.row_capacity,
                "rows ({}) bigger than capacity ({})",
                new_metadata.rows_written,
                new_metadata.row_capacity
            );
            assert!(
                original_metadata.rows_written <= new_metadata.rows_written,
                "original rows ({}) are bigger than new rows ({}) after append",
                original_metadata.rows_written,
                new_metadata.rows_written
            );
            assert!(
                original_metadata.row_capacity <= new_metadata.row_capacity,
                "original row capacity ({}) is bigger than new row capacity ({}) after append",
                original_metadata.row_capacity,
                new_metadata.row_capacity
            );

            // check that the file is correct and correctly decodes to what we expect
            file_handler.verify_all_files_agree().unwrap();
        }
        file_handler.delete_all_files().unwrap();
    }

    fn _safe_delete_file(target: &impl AsRef<Path>) {
        if target.as_ref().exists() {
            remove_file(target).unwrap();
        }
    }

    fn _are_two_files_same(path_a: &impl AsRef<Path>, path_b: &impl AsRef<Path>) -> bool {
        if path_a.as_ref() == path_b.as_ref() {
            return true;
        }
        if (path_a.as_ref().exists() && !path_b.as_ref().exists())
            || (!path_a.as_ref().exists() && path_b.as_ref().exists())
        {
            return false;
        }

        const BUFSIZE: usize = 2usize.pow(6);
        let mut file_a = File::open(path_a).unwrap();
        let mut file_b = File::open(path_b).unwrap();
        let mut buffer_a = vec![0u8; BUFSIZE];
        let mut buffer_b = vec![0u8; BUFSIZE];

        loop {
            let bytes_a = file_a.read(&mut buffer_a).unwrap();
            let bytes_b = file_b.read(&mut buffer_b).unwrap();

            if bytes_a == 0 && bytes_b == 0 {
                break;
            }

            if bytes_a != bytes_b {
                return false;
            }
            if buffer_a != buffer_b {
                return false;
            }
        }
        true
    }
}
