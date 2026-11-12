use clap::Parser;
use jamjam::Cli;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jamjam=info".into()),
        )
        .with_target(false)
        .init();

    if let Err(error) = jamjam::run(Cli::parse()).await {
        eprintln!("jamjam: {error}");
        std::process::exit(1);
    }
}
