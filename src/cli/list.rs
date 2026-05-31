use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub async fn run(_reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = PackageRow::list(&db)?;
    if rows.is_empty() {
        println!("  (no packages installed)");
        return Ok(());
    }
    for row in rows {
        let marker = if row.requested { "●" } else { " " };
        println!("  {marker}  {:<24} {}", row.name, row.version);
    }
    Ok(())
}
