use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::time::Duration;

pub async fn run(full: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    config.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;

    let db = Db::open(&config)?;
    let installed = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let total = installed.len();
    let mut refreshed = 0usize;
    for row in &installed {
        reporter.status(&format!("Refreshing {} ({}/{total})", row.name, refreshed + 1));
        match client.fetch(&row.name, FetchPolicy::ForceRefresh).await {
            Ok(_) => refreshed += 1,
            Err(OlmaError::Network(e)) => {
                reporter.status(&format!("  network: {e} (skipped)"));
            }
            Err(e) => return Err(e),
        }
    }

    if full {
        reporter.status("Fetching full formula dump");
        fetch_full_dump(&config).await?;
    }

    reporter.success(&format!(
        "Refreshed {refreshed}/{total} installed formulae{}",
        if full { " + full index" } else { "" }
    ));
    Ok(())
}

async fn fetch_full_dump(config: &Config) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    let resp = http.get("https://formulae.brew.sh/api/formula.json").send().await
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(OlmaError::Network(format!("formula.json returned {}", resp.status())));
    }
    let body = resp.bytes().await.map_err(|e| OlmaError::Network(e.to_string()))?;
    let target = config.cache_formulae().join("__index.json");
    std::fs::create_dir_all(target.parent().unwrap())?;
    std::fs::write(&target, &body)?;
    Ok(())
}
