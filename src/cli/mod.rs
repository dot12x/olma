pub mod add;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

use crate::metadata::client::FetchPolicy;
use crate::output::default_reporter;

#[derive(Parser)]
#[command(name = "olma", version, about = "macOS package manager")]
pub struct Cli {
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    #[arg(long, global = true)]
    pub dry_run: bool,
    #[arg(long, global = true)]
    pub offline: bool,
    #[arg(long, global = true)]
    pub no_cache: bool,
    #[arg(long, global = true)]
    pub refresh: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Add {
        #[arg(required = true)]
        names: Vec<String>,
    },
}

impl Cli {
    pub fn fetch_policy(&self) -> std::result::Result<FetchPolicy, &'static str> {
        match (self.offline, self.no_cache, self.refresh) {
            (true, true, _) => Err("--offline and --no-cache are mutually exclusive"),
            (true, _, true) => Err("--offline and --refresh are mutually exclusive"),
            (true, _, _) => Ok(FetchPolicy::OfflineOnly),
            (_, true, _) => Ok(FetchPolicy::BypassCache),
            (_, _, true) => Ok(FetchPolicy::ForceRefresh),
            _ => Ok(FetchPolicy::CacheFirst),
        }
    }
}

pub async fn run() -> ExitCode {
    let cli = Cli::parse();
    let reporter = default_reporter();
    let policy = match cli.fetch_policy() {
        Ok(p) => p,
        Err(msg) => {
            reporter.error(msg);
            return ExitCode::from(2);
        }
    };
    let result = match cli.cmd {
        Cmd::Add { names } => add::run(&names, policy, cli.yes, cli.dry_run, reporter.as_ref()).await,
    };
    match result {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            reporter.error(&e.to_string());
            e.exit_code()
        }
    }
}
