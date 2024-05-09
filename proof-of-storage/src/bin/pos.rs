use std::io::BufRead;
use std::net::IpAddr;

use clap::{Parser, Subcommand};

use proof_of_storage::file_metadata::*;
use proof_of_storage::networking::client::*;
use proof_of_storage::networking::server::server_main;

#[derive(Parser, Debug)]
#[command(version, about, arg_required_else_help = true, propagate_version = true)]
struct PosServerOpts {
    /// Subcommands for the server
    #[command(subcommand)]
    subcommand: Option<PoSSubCommands>,

    /// verbosity
    #[arg(global = true)]
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
// #[command(version, about, long_about = None)]
enum PoSSubCommands {
    /// Upload a new file to a remote server
    #[clap(alias = "up")]
    Upload {
        /// The file to upload
        #[clap(required = true)]
        file: std::path::PathBuf,

        /// the server IP to upload the file to
        #[clap(short, long, default_value = "0.0.0.0")]
        ip: Option<std::net::IpAddr>,

        /// the server port to upload the file to
        #[clap(short, long, default_value = "8080")]
        port: Option<u16>,

        /// The number of columns to encode the file with
        #[clap(short, long, default_value_t = 10)]
        columns: usize,
    },

    /// Download an existing file from a remote server
    #[clap(alias = "down")]
    Download,

    /// Request a proof of storage for a file
    #[clap(alias = "pf")]
    Proof {
        /// name of the file to download
        #[clap(required = true)]
        file: String,

        /// the server IP to upload the file to
        #[clap(short, long, default_value = "0.0.0.0")]
        ip: Option<std::net::IpAddr>,

        /// the server port to upload the file to
        #[clap(short, long, default_value = "8080")]
        port: Option<u16>,

        /// the bits of security to use as an override #UNIMIPLEMENTED todo
        #[clap(short, long)]
        security_bits: Option<u8>,
    },

    /// List files you currently have stored on remote servers
    #[clap(alias = "ls")]
    List,

    /// host your own server
    #[clap(alias = "s")]
    Server {
        /// The port to host the server on
        #[clap(short, long, default_value_t = 8080)]
        port: u16,

    },
}

fn is_client_command(subcommand: &PoSSubCommands) -> bool {
    match subcommand {
        PoSSubCommands::Server { .. } => false,
        _ => true,
    }
}


#[tokio::main]
async fn main() {
    let args = PosServerOpts::parse();
    let subcommand = args.subcommand.unwrap();
    let verbosity = args.verbose;

    let _ = start_tracing(&verbosity, &subcommand).or_else(|e| {
        println!("failed to start tracing server: {:?}", e);
        Ok::<(), Box<dyn std::error::Error>>(())
    });

    match subcommand {
        PoSSubCommands::Upload { file, ip, port, columns } => {
            tracing::debug!("uploading file");
            upload_file_command(file, ip, port, columns).await;
        }
        PoSSubCommands::Download => {
            tracing::debug!("downloading file");
        }
        PoSSubCommands::Proof { file, ip, port, security_bits } => {
            tracing::info!("requesting proof of storage");
            tracing::debug!("fetching file metadata from database");
            let file_metadata = get_client_metadata_from_database_by_filename("file_database".to_string(), file).await;

            if file_metadata.is_none() {
                tracing::error!("file not found in database");
                return;
            }

            tracing::debug!("found file metadata: {:?}", &file_metadata.clone().unwrap());

            tracing::debug!("requesting proof from server");
            request_proof_command(file_metadata.unwrap(), ip, port, security_bits).await;
        }
        PoSSubCommands::List => {
            list_files().await;
        }
        PoSSubCommands::Server {
            port, ..
        } => {
            let server_result = server_main(port, verbosity).await;
            if let Err(e) = server_result {
                tracing::error!("server error: {}", e);
            }
            tracing::info!("server terminated");
        }
    }
}

fn start_tracing(verbosity: &u8, subcommand: &PoSSubCommands) -> Result<(), Box<dyn std::error::Error>> {
    let max_level = match verbosity {
        0 => tracing::Level::ERROR,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE, //3+
    };

    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_max_level(max_level)
        .finish();

    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}


async fn upload_file_command(file: std::path::PathBuf, ip: Option<std::net::IpAddr>, port: Option<u16>, columns: usize) {
    let file_name = file.to_str().unwrap().to_string();
    let server_port = port.unwrap();
    let server_ip = ip.unwrap().to_string() + ":" + &server_port.to_string();
    let (file_metadata, root) = proof_of_storage::networking::client::upload_file(file_name, columns, server_ip).await.unwrap();
    tracing::info!("File upload successful");
    tracing::debug!("File Metadata: {:?}", file_metadata);
    tracing::debug!("Root: {:?}", root);

    //todo: need to request proof from server

    // add file to file database for the `list` command
    append_client_file_metadata_to_database("file_database".to_string(), file_metadata.clone()).await.expect("failed to append file metadata to database");
    tracing::info!("appended {} to filedatabase", file_metadata.filename);
}

async fn list_files() {
    let file_database_filename = "file_database".to_string();
    tracing::debug!("reading files from {}", file_database_filename);
    let (hosts_database, file_metadata_database) = read_client_file_database_from_disk(file_database_filename).await;
    println!("files:");
    for file in file_metadata_database { println!("{}", file); }
    println!("\nhosts:");
    for host in hosts_database { println!("{}", host); }
}

async fn request_proof_command(file_metadata: ClientOwnedFileMetadata, ip: Option<IpAddr>, port: Option<u16>, security_bits: Option<u8>) {
    let server_ip = if ip.is_some() { ip.unwrap().to_string() } else {
        file_metadata.stored_server.server_ip.clone()
    };
    let server_port = if port.is_some() { port.unwrap() } else {
        file_metadata.stored_server.server_port.clone()
    };
    let server_ip = server_ip + ":" + &server_port.to_string();
    let security_bits = security_bits.unwrap_or(0);

    let proof = proof_of_storage::networking::client::request_proof(file_metadata, server_ip, security_bits).await.unwrap();
    tracing::info!("Proof received: {:?}", proof);
}
