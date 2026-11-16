use std::process;

use clap::Parser;
use jamparty::Cli;

#[tokio::main]
async fn main() {
    jamparty::init_logging();
    if let Err(error) = jamparty::run(Cli::parse()).await {
        eprintln!("jamparty: error: {}", jamparty::describe(&error));
        process::exit(1);
    }
}
