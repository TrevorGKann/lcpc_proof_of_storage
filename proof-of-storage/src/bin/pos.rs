use clap::{Parser, Subcommand};

use proof_of_storage::networking::server::server_main;

#[derive(Parser, Debug)]
#[command(version, about, arg_required_else_help = true, propagate_version = true)]
struct PosServerOpts {
    /// Subcommands for the server
    #[command(subcommand)]
    subcommand: Option<ClientSubCommands>,
}

#[derive(Subcommand, Debug)]
// #[command(version, about, long_about = None)]
enum ClientSubCommands {
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

        /// verbosity level
        #[clap(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },
}

#[tokio::main]
async fn main() {
    let args = PosServerOpts::parse();
    let subcommand = args.subcommand.unwrap();
    match subcommand {
        ClientSubCommands::Upload { file, ip, port, columns } => {
            println!("uploading file");
            upload_file_command(file, ip, port, columns).await;
        }
        ClientSubCommands::Download => {
            println!("downloading file");
        }
        ClientSubCommands::Proof => {
            println!("requesting proof of storage");
        }
        ClientSubCommands::List => {
            println!("listing files");
        }
        ClientSubCommands::Server { port, verbose } => {
            let server_result = server_main(port, verbose).await;
            if let Err(e) = server_result {
                eprintln!("server error: {}", e);
            }
            println!("server terminated");
        }
    }
}


async fn upload_file_command(file: std::path::PathBuf, ip: Option<std::net::IpAddr>, port: Option<u16>, columns: usize) {
    let file_name = file.to_str().unwrap().to_string();
    let server_port = port.unwrap();
    let server_ip = ip.unwrap().to_string() + ":" + &server_port.to_string();
    let (file_metadata, root) = proof_of_storage::networking::client::upload_file(file_name, columns, server_ip).await.unwrap();
    println!("File upload successful");
    println!("File Metadata: {:?}", file_metadata);
    println!("Root: {:?}", root);
}