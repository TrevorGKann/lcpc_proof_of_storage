use std::fmt::Display;
use std::path::PathBuf;
use tokio::runtime::{Builder, Runtime};
use tracing_subscriber::EnvFilter;

pub fn start_bench_subscriber() {
    let env_filter: EnvFilter = EnvFilter::builder()
        .parse("surrealdb_core=warn,surrealdb=warn,trace")
        .unwrap();

    let subscriber = tracing_subscriber::fmt()
        .pretty()
        .compact()
        .with_file(true)
        .with_line_number(true)
        .with_env_filter(env_filter)
        .with_max_level(tracing::Level::DEBUG)
        .finish();

    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber).unwrap();
}

pub fn get_bench_runtime() -> Runtime {
    Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .expect("couldn't create asynchronous runtime")
}

pub fn get_bench_single_use_subdir(name: &impl Display) -> PathBuf {
    let test_dir = PathBuf::from(format!("bench_files/{name}"));
    std::fs::create_dir_all(&test_dir).expect("couldn't create test directory");
    test_dir
}
