use std::process;

use clap::Parser;
use jampot::Cli;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "jampot=info".into()))
        .with_target(false)
        .init();

    if let Err(error) = jampot::run(Cli::parse()).await {
        eprintln!("jampot: {}", jampot::describe(&error));
        process::exit(1);
    }
}
