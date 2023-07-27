use clap::{Parser, ValueEnum};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Problem {
    SmokeTest,
    PrimeTime,
}

#[derive(Parser, Debug)]
struct Args {
    /// Host and port to listen on
    #[arg(short, long, default_value = "0.0.0.0:32767")]
    listen: SocketAddr,

    /// Problem to run
    #[arg(short, long, default_value = "prime-time")]
    problem: Problem,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    tracing_subscriber::fmt::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    info!(listener=%args.listen, "listening");

    let listener = TcpListener::bind(args.listen).await?;

    match args.problem {
        Problem::SmokeTest => smoke_test::serve(listener).await,
        Problem::PrimeTime => prime_time::serve(listener).await,
    };

    Ok(())
}
