use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, transactions::TransactionRow};

pub async fn run(limit: i64, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = TransactionRow::list(&db, limit)?;
    if rows.is_empty() {
        println!("  (no transactions yet)");
        return Ok(());
    }
    for row in rows {
        let changes = row.parse_changes()?;
        let names: Vec<String> = changes.iter().map(|c| {
            match (&c.from_version, &c.to_version) {
                (None, Some(to)) => format!("{} {}", c.name, to),
                (Some(from), Some(to)) => format!("{} {}→{}", c.name, from, to),
                (Some(from), None) => format!("{}- {}", c.name, from),
                (None, None) => c.name.clone(),
            }
        }).collect();
        let r = if row.reverted { " (reverted)" } else { "" };
        println!("  #{:<4}  {:<8}  {}{}", row.id, row.kind, names.join(", "), r);
    }
    Ok(())
}
