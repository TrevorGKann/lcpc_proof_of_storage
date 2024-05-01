use clap::{Parser, Subcommand};

use proof_of_storage::file_metadata::{ClientOwnedFileMetadata, read_client_file_database_from_disk, ServerHost, write_client_file_database_to_disk};
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
    Proof,

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

fn isClientCommand(subcommand: &PoSSubCommands) -> bool {
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

    start_tracing(&verbosity, &subcommand).or_else(|e| {
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
        PoSSubCommands::Proof => {
            tracing::debug!("requesting proof of storage");
        }
        PoSSubCommands::List => {
            list_files().await;
        }
        PoSSubCommands::Server { port } => {
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
        3 => tracing::Level::TRACE,
        2 => tracing::Level::DEBUG,
        1 => tracing::Level::INFO,
        _ => tracing::Level::ERROR,
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

    let (mut hosts_database, mut file_metadata_database) = read_client_file_database_from_disk("file_database".to_string()).await;
    hosts_database.push(file_metadata.stored_server.clone());
    file_metadata_database.push(file_metadata);
    write_client_file_database_to_disk("file_database".to_string(), hosts_database, file_metadata_database).await;
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