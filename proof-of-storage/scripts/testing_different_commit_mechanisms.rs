use anyhow::Result;
use blake3::Hasher as Blake3;
use human_hash::humanize;
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::field_generator_iter::FieldGeneratorIter;
use proof_of_storage::fields::{Ft253_192, WriteableFt63};
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::row_generator_iter::RowGeneratorIter;
use rand::seq::index::sample;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fmt::Display;
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::ops::{Add, Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use ulid::Ulid;
use uuid::Uuid;

// yeah, this is jank but whatever
#[path = "../../benches/bench_utils.rs"]
mod bench_utils;

type BenchField = Ft253_192;
type BenchDigest = Blake3;
type BenchFileHandler = FileHandler<BenchDigest, BenchField, LigeroEncoding<BenchField>>;

static RESULTS_DICT: LazyLock<Mutex<HashMap<String, HashMap<String, String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn main() -> Result<()> {
    bench_utils::start_bench_subscriber();

    // test setup
    // let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    let test_file_path = PathBuf::from("test_files/10000_byte_file.bytes");
    let total_file_bytes = std::fs::metadata(&test_file_path)?.len();
    let test_ulid = Ulid::new();
    let test_identifier = humanize(&Uuid::from(test_ulid), 3);
    let working_dir = env::current_dir()?
        .join("test_results")
        .join(&test_identifier);
    create_dir_all(&working_dir)?;
    tracing::info!("starting test {test_identifier}");

    // test parameters
    let pow_2 = 19;
    let pre_encoded_len = 2usize.pow(pow_2);
    let encoded_len = 2usize.pow(pow_2 + 1);
    add_result_metadata("preencoded len", format!("{pre_encoded_len}"));
    add_result_metadata("encoded len", format!("{encoded_len}"));
    add_result_metadata("original file length", format!("{total_file_bytes}"));
    add_result_metadata("ulid", format!("{test_ulid}"));

    // execute the tests
    let file_handler = full_commit(
        pre_encoded_len,
        encoded_len,
        &test_file_path,
        &working_dir,
        &test_ulid,
    )?;

    // get columns from full commit
    get_cols_from_full(&file_handler);

    get_cols_from_stream(&test_file_path, pre_encoded_len, encoded_len);

    // save results
    let results_string = serde_json::to_string_pretty(&RESULTS_DICT.lock().unwrap().deref())?;
    tracing::info!("Results:\n{results_string}");
    File::create(working_dir.join("results.json"))?.write_all(results_string.as_bytes())?;
    Ok(())
}

fn full_commit(
    pre_encoded_len: usize,
    encoded_len: usize,
    source_file: &PathBuf,
    dir: &PathBuf,
    ulid: &Ulid,
) -> Result<BenchFileHandler> {
    tracing::info!("encoding with rate {pre_encoded_len}/{encoded_len}");

    // file setup
    let unencoded_test_file = dir.join(format!(
        "{}.{}",
        ulid.to_string(),
        constants::UNENCODED_FILE_EXTENSION
    ));
    std::fs::copy(source_file, &unencoded_test_file).expect("couldn't copy test source file");
    let mut total_duration = Duration::from_secs(0);

    // the test
    let start = Instant::now();
    let file_handler = BenchFileHandler::create_from_unencoded_file(
        &ulid,
        Some(&unencoded_test_file),
        pre_encoded_len,
        encoded_len,
    )
    .expect("couldn't open the encoded file");
    total_duration = total_duration.add(start.elapsed());
    add_result_data("full commit", "time", format!("{total_duration:?}"));

    Ok(file_handler)
}

fn get_cols_from_full(file_handler: &BenchFileHandler) {
    todo!()
}

fn get_cols_from_stream(target_file: &PathBuf, pre_encoded_len: usize, encoded_len: usize) {
    let buf_reader = std::io::BufReader::new(std::fs::File::open(&target_file).unwrap())
        .bytes()
        .map(|b| b.unwrap());
    let field_iterator = FieldGeneratorIter::<_, WriteableFt63>::new(buf_reader);
    let row_iterator = RowGeneratorIter::new_ligero(field_iterator, pre_encoded_len, encoded_len);
    let streamed_root = row_iterator.convert_to_commit_root::<Blake3>();

    todo!()
}

fn add_result_metadata(key: impl ToString, value: impl ToString) {
    let mut dict = RESULTS_DICT.lock().unwrap();
    if !dict.contains_key("parameters") {
        dict.insert("parameters".to_string(), HashMap::new());
    }
    dict.get_mut("parameters")
        .unwrap()
        .insert(key.to_string(), value.to_string());
}

fn add_result_data(test_key: impl ToString, key: impl ToString, value: impl ToString) {
    let mut dict = RESULTS_DICT.lock().unwrap();
    if !dict.contains_key(&test_key.to_string()) {
        dict.insert(test_key.to_string(), HashMap::new());
    }
    dict.get_mut(&test_key.to_string())
        .unwrap()
        .insert(key.to_string(), value.to_string());
}
