use blake3::Hasher as Blake3;
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;
use tokio::fs;

use proof_of_storage::fields::{convert_byte_vec_to_field_elements_vec, writable_ft63};
use proof_of_storage::fields::writable_ft63::WriteableFt63;
use proof_of_storage::file_metadata::{ClientOwnedFileMetadata, ServerHost, ServerOwnedFileMetadata};
use proof_of_storage::lcpc_online::{CommitDimensions, CommitOrLeavesOutput, CommitRequestType, convert_file_data_to_commit};

#[derive(Debug, Serialize, Deserialize)]
struct Name {
    first: String,
    last: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Person {
    is_male: bool,
    name: Name,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create database connection
    let db = Surreal::new::<RocksDb>("PoR_Database").await?;
    db.use_ns("server").use_db("server").await?;

    let mut file_data = fs::read("/home/trevor/Research/PoR/lcpc/proof-of-storage/test_files/test.txt").await?;
    let file_field_data = convert_byte_vec_to_field_elements_vec(&file_data);
    let CommitOrLeavesOutput::Commit(commit) = convert_file_data_to_commit::<Blake3, WriteableFt63>(
        &file_field_data,
        CommitRequestType::Commit,
        CommitDimensions::Square,
    )? else { panic!("failed to convert file data to commit") };

    let server_data = ServerOwnedFileMetadata {
        filename: "test".to_string(),
        owner: "myself".to_string(),
        commitment: commit,
    };

    let created = db.create::<Vec<ServerOwnedFileMetadata>>("server_file_metadata")
        .content(server_data.clone()).await?;

    // println!("created: {created:?}");


    let my_result = db.select::<Vec<ServerOwnedFileMetadata>>("server_file_metadata").await?;

    // println!("result of query: {:?}", my_result);

    assert_eq!(server_data.filename, my_result[0].filename);
    assert_eq!(server_data.owner, my_result[0].owner);
    assert_eq!(server_data.commitment.comm, my_result[0].commitment.comm);
    assert_eq!(server_data.commitment.coeffs, my_result[0].commitment.coeffs);
    assert_eq!(server_data.commitment.n_rows, my_result[0].commitment.n_rows);

    Ok(())
}