#[cfg(test)]
use crate::lcpc_online::encoded_file_reader::EncodedFileReader;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use crate::lcpc_online::*;
use std::fs::File;
use std::path::PathBuf;

#[tokio::test]
async fn encode_then_decode_file() {
    let file_path = PathBuf::from("test_files/test.txt");
    let file_path_encoded = PathBuf::from("test_files/test_encoded.txt");
    let file_path_decoded = PathBuf::from("test_files/test_decoded.txt");

    println!(
        "current dir: {:}",
        std::env::current_dir().unwrap().display()
    );
    println!("test file: {:}", file_path.display());

    let original_file_len = File::open(&file_path).unwrap().metadata().unwrap().len() as usize;
    let pre_encoded_len = 8usize;
    let encoded_len = 16usize;

    let encoded_tree = EncodedFileWriter::<WriteableFt63, Blake3, LigeroEncoding<WriteableFt63>>::convert_unencoded_file(
        &mut tokio::fs::File::open(&file_path).await.expect("couldn't open test file"),
        &file_path_encoded,
        None,
        pre_encoded_len,
        encoded_len,
    )
        .await
        .expect("failed initial encoding");
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

    let encoded_file = tokio::fs::OpenOptions::default()
        .read(true)
        .write(true)
        .open(&file_path_encoded)
        .await
        .expect("couldn't open encoded test file");
    let mut reader =
        EncodedFileReader::<WriteableFt63, Blake3, LigeroEncoding<WriteableFt63>>::new_ligero(
            encoded_file,
            pre_encoded_len,
            encoded_len,
        )
        .await;
    let mut decode_target = tokio::fs::OpenOptions::default()
        .read(true)
        .write(true)
        .create(true)
        .open(&file_path_decoded)
        .await
        .expect("couldn't open decode target");
    reader
        .decode_to_target_file(&mut decode_target)
        .await
        .expect("couldn't decode encoded target");

    // check that file_path and file_path_decoded are equal
    let file_path_contents = tokio::fs::read(&file_path).await.unwrap();
    let file_path_decoded_contents = tokio::fs::read(&file_path_decoded).await.unwrap();
    assert_eq!(
        file_path_contents[..],
        file_path_decoded_contents[..file_path_contents.len()]
    );
}
