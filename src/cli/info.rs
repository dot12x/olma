use crate::config::Config;
use crate::error::Result;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use walkdir::WalkDir;

pub async fn run(name: &str, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let client = FormulaeClient::new(&config)?;
    let formula = client.fetch(name, FetchPolicy::CacheFirst).await?;
    let db = Db::open(&config)?;
    let installed = PackageRow::get(&db, name)?;

    println!("  {}", formula.name);
    if let Some(desc) = &formula.desc {
        println!("  {desc}");
    }
    if let Some(home) = &formula.homepage {
        println!("  {home}");
    }
    println!();
    println!("  Version:    {}", formula.version());
    if let Some(row) = &installed {
        let marker = if row.requested { "●" } else { " " };
        println!("  Installed:  {marker}  {} (installed {})", row.version, fmt_ts(row.installed_at));
        if let Some(prev) = &row.previous_ver {
            println!("  Previous:   {prev}  (rollback target)");
        }
        let dir = config.package_dir(name, &row.version);
        let size = dir_size(&dir);
        println!("  Size:       {}", human(size));
    } else {
        println!("  Installed:  not installed");
    }
    if !formula.dependencies.is_empty() {
        println!("  Deps:       {}", formula.dependencies.join(", "));
    }
    Ok(())
}

fn dir_size(p: &std::path::Path) -> u64 {
    if !p.exists() { return 0; }
    WalkDir::new(p).into_iter().filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn human(b: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 { v /= 1024.0; u += 1; }
    format!("{v:.1} {}", UNITS[u])
}

fn fmt_ts(ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::from_secs(ts.max(0) as u64);
    let now = std::time::SystemTime::now();
    let delta = now.duration_since(t).unwrap_or_default().as_secs();
    if delta < 60 { return format!("{delta}s ago"); }
    if delta < 3600 { return format!("{}m ago", delta / 60); }
    if delta < 86400 { return format!("{}h ago", delta / 3600); }
    format!("{}d ago", delta / 86400)
}
