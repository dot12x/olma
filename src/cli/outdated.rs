use crate::config::Config;
use crate::error::Result;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub struct OutdatedRow {
    pub name: String,
    pub installed: String,
    pub latest: String,
}

pub async fn run(_reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let installed = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let mut rows = Vec::new();
    for pkg in &installed {
        let formula = match client.fetch(&pkg.name, FetchPolicy::CacheFirst).await {
            Ok(f) => f,
            Err(_) => continue,
        };
        if formula.version() != pkg.version {
            rows.push(OutdatedRow {
                name: pkg.name.clone(),
                installed: pkg.version.clone(),
                latest: formula.version().to_string(),
            });
        }
    }

    if rows.is_empty() {
        println!("  (everything up to date)");
        return Ok(());
    }
    println!("  {:<24} {:<14} → {}", "NAME", "INSTALLED", "LATEST");
    for r in &rows {
        println!("  {:<24} {:<14} → {}", r.name, r.installed, r.latest);
    }
    println!("\n  Run `olma upgrade` to upgrade all, or `olma upgrade <name>` for one.");
    Ok(())
}
