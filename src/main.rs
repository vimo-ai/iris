//! iris - LSP-powered code analysis toolkit

mod akin_cli;
mod arch_cli;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "iris")]
#[command(about = "LSP-powered code analysis toolkit", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Code similarity detection
    #[command(subcommand)]
    Akin(akin_cli::AkinCommands),
    /// Architecture analysis
    #[command(subcommand)]
    Arch(arch_cli::ArchCommands),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Akin(cmd) => akin_cli::run(cmd).await?,
        Commands::Arch(cmd) => arch_cli::run(cmd).await?,
    }

    Ok(())
}
