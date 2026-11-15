use std::process;

use clap::Parser;
use jampot::Cli;

#[tokio::main]
async fn main() {
    jampot::init_logging();
    if let Err(error) = jampot::run(Cli::parse()).await {
        eprintln!("jampot: error: {}", jampot::describe(&error));
        process::exit(1);
    }
}
