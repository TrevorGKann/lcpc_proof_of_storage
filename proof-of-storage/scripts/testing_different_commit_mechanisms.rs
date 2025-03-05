#![allow(unused)]
use anyhow::Result;
use blake3::Hasher as Blake3;
use clap::Parser;
use human_hash::humanize;
use lcpc_ligero_pc::LigeroEncoding;
use proof_of_storage::databases::constants;
use proof_of_storage::fields::field_generator_iter::FieldGeneratorIter;
use proof_of_storage::fields::{Ft253_192, WriteableFt63};
use proof_of_storage::lcpc_online::column_digest_accumulator::ColumnsToCareAbout;
use proof_of_storage::lcpc_online::file_formatter::get_unencoded_file_location_from_id;
use proof_of_storage::lcpc_online::file_handler::FileHandler;
use proof_of_storage::lcpc_online::row_generator_iter::RowGeneratorIter;
use proof_of_storage::lcpc_online::{_get_POS_soundness_n_cols, convert_file_data_to_commit};
use proof_of_storage::networking::client::get_column_indicies_from_random_seed;
use proof_of_storage::networking::server::get_aspect_ratio_default_from_file_len;
use rand::seq::index::sample;
use rand_chacha::ChaChaRng;
use rand_core::{RngCore, SeedableRng};
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

#[derive(Parser, Debug)]
struct Args {
    #[arg(short = 'd', trailing_var_arg = true)]
    additional_data: Vec<String>,
}
type BenchField = Ft253_192;
type BenchDigest = Blake3;
type BenchFileHandler = FileHandler<BenchDigest, BenchField, LigeroEncoding<BenchField>>;

static RESULTS_DICT: LazyLock<Mutex<HashMap<String, HashMap<String, String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn main() -> Result<()> {
    bench_utils::start_bench_subscriber();
    if PathBuf::from("proof-of-storage").is_dir() {
        env::set_current_dir(PathBuf::from("proof-of-storage"))?;
    }
    tracing::debug!(
        "current directory is {}",
        env::current_dir().unwrap().display()
    );

    // test setup
    let test_file_path = PathBuf::from("test_files/1000000000_byte_file.bytes");
    // let test_file_path = PathBuf::from("test_files/10000000_byte_file.bytes");
    // let test_file_path = PathBuf::from("test_files/10000_byte_file.bytes");
    tracing::debug!("opening test_file {:?}", test_file_path);
    let total_file_bytes = std::fs::metadata(&test_file_path)?.len();
    let test_ulid = Ulid::new();
    let current_time = chrono::Local::now().format("%y-%m-%d-%H:%M:%S").to_string();
    let test_identifier = humanize(&Uuid::from(test_ulid), 3);
    let file_tag = format!("{}-{}", current_time, test_identifier);
    let working_dir = env::current_dir()?.join("test_results").join(&file_tag);
    create_dir_all(&working_dir)?;
    tracing::info!("starting test {test_identifier}");

    // test parameters
    let args = Args::parse();

    // let pow_2 = 16;
    // let pre_encoded_len = 2usize.pow(pow_2);
    // let encoded_len = 2usize.pow(pow_2 + 1);
    let (pre_encoded_len, encoded_len, _) =
        get_aspect_ratio_default_from_file_len::<BenchField>(total_file_bytes as usize);

    add_result_metadata("preencoded len", format!("{pre_encoded_len}"));
    add_result_metadata("encoded len", format!("{encoded_len}"));
    add_result_metadata("original file length", format!("{total_file_bytes}"));
    add_result_metadata("ulid", format!("{test_ulid}"));
    add_result_metadata("time", current_time);
    add_result_metadata("id", test_identifier);
    if !args.additional_data.is_empty() {
        add_result_metadata("additional data", args.additional_data.join(","));
    }

    // execute the tests
    let mut file_handler = full_commit(
        pre_encoded_len,
        encoded_len,
        &test_file_path,
        &working_dir,
        &test_ulid,
    )?;

    // get columns from the full commit on the disk
    get_cols_from_full(&mut file_handler);

    // get collumns without ever using the disk
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
    dir: &Path,
    ulid: &Ulid,
) -> Result<BenchFileHandler> {
    tracing::info!("encoding with rate {pre_encoded_len}/{encoded_len}");

    // file setup
    let unencoded_test_file = get_unencoded_file_location_from_id(&ulid);
    // let unencoded_test_file = dir.join("").join(format!(
    //     "{}.{}",
    //     ulid.to_string(),
    //     constants::UNENCODED_FILE_EXTENSION
    // ));
    dbg!(&unencoded_test_file);
    std::fs::copy(source_file, &unencoded_test_file).expect("couldn't copy test source file");
    let mut total_duration = Duration::from_secs(0);

    // the test
    let start = Instant::now();
    let file_handler = BenchFileHandler::create_from_unencoded_file(
        &ulid,
        Some(&unencoded_test_file),
        // None,
        pre_encoded_len,
        encoded_len,
    )
    .expect("couldn't open the encoded file");
    total_duration = total_duration.add(start.elapsed());
    add_result_data("full commit", "time", format!("{total_duration:?}"));

    Ok(file_handler)
}

fn get_cols_from_full(file_handler: &mut BenchFileHandler) -> Result<()> {
    // the setup
    let (pre_encoded_len, encoded_len, _rows) = file_handler.get_dimensions()?;
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
    let mut rand = ChaChaRng::from_entropy();
    let columns_to_fetch =
        get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

    // the test
    let start = Instant::now();
    let _columns_fetched =
        file_handler.read_full_columns(ColumnsToCareAbout::Only(columns_to_fetch.clone()))?;
    add_result_data(
        "columns from disk commit",
        "time",
        format!("{:?}", start.elapsed()),
    );

    Ok(())
}

fn get_cols_from_stream(target_file: &PathBuf, pre_encoded_len: usize, encoded_len: usize) {
    // the setup
    let num_cols_required = _get_POS_soundness_n_cols(pre_encoded_len, encoded_len);
    let mut rand = ChaChaRng::from_entropy();
    let columns_to_fetch =
        get_column_indicies_from_random_seed(rand.next_u64(), num_cols_required, encoded_len);

    let buf_reader = std::io::BufReader::new(std::fs::File::open(&target_file).unwrap())
        .bytes()
        .map(|b| b.unwrap());
    let field_iterator = FieldGeneratorIter::<_, WriteableFt63>::new(buf_reader);
    let row_iterator = RowGeneratorIter::new_ligero(field_iterator, pre_encoded_len, encoded_len);

    // the test
    let start = Instant::now();
    let streamed_columns = row_iterator.get_full_columns::<Blake3>(&columns_to_fetch);
    add_result_data(
        "columns from streaming",
        "time",
        format!("{:?}", start.elapsed()),
    );
}

fn add_result_metadata(key: impl ToString + Display, value: impl ToString + Display) {
    tracing::debug!("parameter entered {key}:{value}");
    let mut dict = RESULTS_DICT.lock().unwrap();
    if !dict.contains_key("parameters") {
        dict.insert("parameters".to_string(), HashMap::new());
    }
    dict.get_mut("parameters")
        .unwrap()
        .insert(key.to_string(), value.to_string());
}

fn add_result_data(
    test_key: impl ToString + Display,
    key: impl ToString + Display,
    value: impl ToString + Display,
) {
    tracing::debug!("parameter entered {test_key} => {key}:{value}");
    let mut dict = RESULTS_DICT.lock().unwrap();
    if !dict.contains_key(&test_key.to_string()) {
        dict.insert(test_key.to_string(), HashMap::new());
    }
    dict.get_mut(&test_key.to_string())
        .unwrap()
        .insert(key.to_string(), value.to_string());
}
