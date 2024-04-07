use clap::{Parser, ValueEnum};
use std::net::SocketAddr;


use tracing_subscriber::EnvFilter;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Problem {
    SmokeTest,
    PrimeTime,
    MeansToAnEnd,
    BudgetChat,
    UnusualDatabaseProgram,
    MobInTheMiddle,
    SpeedDaemon,
}

#[derive(Parser, Debug)]
struct Args {
    /// Host and port to listen on
    #[arg(short, long, default_value = "0.0.0.0:32767")]
    listen: SocketAddr,

    /// Problem to run
    #[arg(short, long, default_value = "speed-daemon")]
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

    match args.problem {
        Problem::SmokeTest => smoke_test::serve(args.listen).await?,
        Problem::PrimeTime => prime_time::serve(args.listen).await?,
        Problem::MeansToAnEnd => means_to_an_end::serve(args.listen).await?,
        Problem::BudgetChat => budget_chat::serve(args.listen).await?,
        Problem::UnusualDatabaseProgram => unusual_database_program::serve(args.listen).await?,
        Problem::MobInTheMiddle => mob_in_the_middle::serve(args.listen).await?,
        Problem::SpeedDaemon => speed_daemon::serve(args.listen).await?,
    };

    Ok(())
}
