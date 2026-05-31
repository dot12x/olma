pub mod add;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

use crate::output::default_reporter;

#[derive(Parser)]
#[command(name = "olma", version, about = "macOS package manager")]
pub struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Add { name: String },
}

pub async fn run() -> ExitCode {
    let cli = Cli::parse();
    let reporter = default_reporter();
    let result = match cli.cmd {
        Cmd::Add { name } => add::run(&name, reporter.as_ref()).await,
    };
    match result {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            reporter.error(&e.to_string());
            e.exit_code()
        }
    }
}
