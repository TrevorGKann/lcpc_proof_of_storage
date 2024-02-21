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
    Upload,

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
    }
}

#[tokio::main]
async fn main() {
    let args = PosServerOpts::parse();
    let subcommand = args.subcommand.unwrap();
    match subcommand {
        ClientSubCommands::Upload => {
            println!("uploading file");
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
        ClientSubCommands::Server{port, verbose} => {
            let server_result = server_main(port, verbose).await;
            if let Err(e) = server_result {
                eprintln!("server error: {}", e);
            }
            println!("server terminated");
        }
    }
}