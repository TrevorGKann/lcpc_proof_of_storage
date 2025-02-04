#[cfg(test)]
mod encoded_file_io_tests {
    use crate::lcpc_online::encoded_file_reader::EncodedFileReader;
    use crate::lcpc_online::encoded_file_reader::{
        get_decoded_file_size_from_rate, get_encoded_file_size_from_rate,
    };
    use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
    use crate::lcpc_online::file_handler::FileHandler;
    use crate::lcpc_online::*;
    use rand::Rng;
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    use serial_test::serial;
    use std::fs::File;
    use std::path::PathBuf;
    use tokio::fs;
    use ulid::Ulid;

    #[tokio::test]
    #[serial]
    async fn encode_then_decode_file() {
        use crate::fields::data_field::DataField;
        // use ff::Field;
        let test_file_path = PathBuf::from("test_files/test.txt");
        let file_path_encoded = PathBuf::from("test_files/test_encoded.txt");
        let file_path_decoded = PathBuf::from("test_files/test_decoded.txt");

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
            let encoded_tree = EncodedFileWriter::<
                WriteableFt63,
                Blake3,
                LigeroEncoding<WriteableFt63>,
            >::convert_unencoded_file(
                &mut tokio::fs::File::open(&test_file_path)
                    .await
                    .expect("couldn't open test file"),
                &file_path_encoded,
                None,
                pre_encoded_len,
                encoded_len,
            )
            .await
            .expect("failed initial encoding");

            // check that the results are the correct size
            assert_eq!(encoded_tree.len(), encoded_len * 2 - 1);
            let encoded_file_len = File::open(&file_path_encoded)
                .unwrap()
                .metadata()
                .unwrap()
                .len() as usize;

            let expected_size = original_file_len
                .div_ceil(WriteableFt63::DATA_BYTE_CAPACITY as usize)
                .div_ceil(pre_encoded_len)
                * WriteableFt63::WRITTEN_BYTES_WIDTH as usize
                * encoded_len;
            println!(
                "Encoded file len: {}; expected {}",
                encoded_file_len, expected_size
            );
            assert_eq!(
                encoded_file_len, expected_size,
                "expected a file of size {} to be encoded into size {}",
                original_file_len, expected_size
            );

            // decode the file
            let encoded_file = tokio::fs::OpenOptions::default()
                .read(true)
                .write(true)
                .open(&file_path_encoded)
                .await
                .expect("couldn't open encoded test file");
            let mut reader = EncodedFileReader::<
                WriteableFt63,
                Blake3,
                LigeroEncoding<WriteableFt63>,
            >::new_ligero(encoded_file, pre_encoded_len, encoded_len)
            .await;
            let mut decode_target = tokio::fs::OpenOptions::default()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&file_path_decoded)
                .await
                .expect("couldn't open decode target");
            reader
                .decode_to_target_file(&mut decode_target)
                .await
                .expect("couldn't decode encoded target");

            // check that the original file and the decoded versions are the same
            let file_path_contents = tokio::fs::read(&test_file_path).await.unwrap();
            let file_path_decoded_contents = tokio::fs::read(&file_path_decoded).await.unwrap();
            assert_eq!(
                file_path_contents[..],
                file_path_decoded_contents[..file_path_contents.len()]
            );
        }

        fs::remove_file(file_path_encoded).await.unwrap();
        fs::remove_file(file_path_decoded).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn edit_file_is_correct() {
        let mut random = ChaCha8Rng::from_entropy();
        const RANDOM_LENGTH: usize = 64;

        let test_file_path_origin = PathBuf::from("test_files/test.txt");
        let test_file_path = PathBuf::from("test_files/edit_test.txt");
        let file_path_encoded = PathBuf::from("test_files/edit_test_encoded.txt");
        let file_path_decoded = PathBuf::from("test_files/edit_test_decoded.txt");
        let _file_path_merkle = PathBuf::from("test_files/edit_test_merkle.txt");
        let test_ulid = Ulid::new();

        // // copy origin to test
        // tokio::fs::copy(&test_file_path_origin, &test_file_path)
        //     .await
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
            tokio::fs::copy(&test_file_path_origin, &test_file_path)
                .await
                .unwrap();

            // get the original data
            let mut original_file_contents = tokio::fs::read(&test_file_path).await.unwrap();
            let original_file_len = File::open(&test_file_path)
                .unwrap()
                .metadata()
                .unwrap()
                .len() as usize;

            let mut file_handler = FileHandler::<
                Blake3,
                WriteableFt63,
                LigeroEncoding<WriteableFt63>,
            >::create_from_unencoded_file(
                &test_ulid,
                Some(&test_file_path),
                pre_encoded_len,
                encoded_len,
            )
            .await
            .unwrap();

            file_handler.verify_all_files_agree().await.unwrap();

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
                    .await
                    .unwrap();
                assert_eq!(updated_tree.len(), encoded_len * 2 - 1);
                assert_eq!(original_bytes, returned_original_bytes,
                           "returned bytes weren't correct. For reference the write address was {} and the bytes to write were {:?}", start_index, random_bytes_to_write);

                // update our running tally of what the file should be
                original_file_contents[start_index..start_index + RANDOM_LENGTH]
                    .copy_from_slice(&random_bytes_to_write);

                // check that the file is correct and correctly decodes to what we expect
                file_handler.verify_all_files_agree().await.unwrap();
                let mut reader = EncodedFileReader::<
                    WriteableFt63,
                    Blake3,
                    LigeroEncoding<WriteableFt63>,
                >::new_ligero(
                    tokio::fs::File::open(file_handler.get_encoded_file_handle())
                        .await
                        .unwrap(),
                    pre_encoded_len,
                    encoded_len,
                )
                .await;

                let mut decode_target = tokio::fs::OpenOptions::default()
                    .read(true)
                    .write(true)
                    .truncate(true)
                    .open(&file_handler.get_raw_file_handle())
                    .await
                    .unwrap();
                reader
                    .decode_to_target_file(&mut decode_target)
                    .await
                    .unwrap();

                let decoded_file_contents = tokio::fs::read(&file_handler.get_raw_file_handle())
                    .await
                    .unwrap();
                assert_eq!(
                    decoded_file_contents[..original_file_contents.len()],
                    original_file_contents
                );
            }
        }

        // cleanup
        if test_file_path.exists() {
            tokio::fs::remove_file(test_file_path).await.unwrap();
        }
        if file_path_encoded.exists() {
            tokio::fs::remove_file(file_path_encoded).await.unwrap();
        }
        if file_path_decoded.exists() {
            tokio::fs::remove_file(file_path_decoded).await.unwrap();
        }
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
                let encoded_file_size = get_encoded_file_size_from_rate::<WriteableFt63>(
                    random_file_len,
                    pre_encoded_len,
                    encoded_len,
                );
                let decoded_file_size = get_decoded_file_size_from_rate::<WriteableFt63>(
                    encoded_file_size,
                    pre_encoded_len,
                    encoded_len,
                );
                assert_eq!(
                    random_file_len.next_multiple_of(
                        WriteableFt63::DATA_BYTE_CAPACITY as usize * pre_encoded_len
                    ),
                    decoded_file_size,
                    "failed with an input file size of {}; for reference the encoded size was {}",
                    random_file_len,
                    encoded_file_size
                );
            }
        }
    }
}
