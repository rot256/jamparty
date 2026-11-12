use clap::Parser;
use nabe::Cli;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nabe=info".into()),
        )
        .with_target(false)
        .init();

    if let Err(error) = nabe::run(Cli::parse()).await {
        eprintln!("nabe: {error}");
        std::process::exit(1);
    }
}
