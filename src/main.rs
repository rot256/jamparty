use clap::Parser;
use jampot::Cli;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jampot=info".into()),
        )
        .with_target(false)
        .init();

    if let Err(error) = jampot::run(Cli::parse()).await {
        eprintln!("jampot: {error}");
        std::process::exit(1);
    }
}
