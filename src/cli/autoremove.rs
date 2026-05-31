use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::collections::HashSet;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let mut needed: HashSet<String> = HashSet::new();
    for r in rows.iter().filter(|r| r.requested) {
        match client.fetch(&r.name, FetchPolicy::CacheFirst).await {
            Ok(formula) => {
                for d in &formula.dependencies {
                    needed.insert(d.clone());
                }
                needed.insert(r.name.clone());
            }
            Err(_) => {
                needed.insert(r.name.clone());
            }
        }
    }

    let mut orphans: Vec<&PackageRow> = rows.iter()
        .filter(|r| !r.requested && !needed.contains(&r.name))
        .collect();
    orphans.sort_by(|a, b| a.name.cmp(&b.name));

    if orphans.is_empty() {
        reporter.success("No orphaned packages");
        return Ok(());
    }

    reporter.status(&format!("Will remove {} orphan packages:", orphans.len()));
    for o in &orphans {
        reporter.status(&format!("  {} {}", o.name, o.version));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let names: Vec<String> = orphans.iter().map(|r| r.name.clone()).collect();
    crate::cli::remove::run(&names, true, reporter).await?;
    reporter.success(&format!("Removed {} orphans", names.len()));
    Ok(())
}

pub fn classify_orphans(rows: &[PackageRow], deps_of: &dyn Fn(&str) -> Vec<String>) -> Vec<String> {
    let mut needed: HashSet<String> = HashSet::new();
    for r in rows.iter().filter(|r| r.requested) {
        needed.insert(r.name.clone());
        for d in deps_of(&r.name) {
            needed.insert(d);
        }
    }
    rows.iter()
        .filter(|r| !r.requested && !needed.contains(&r.name))
        .map(|r| r.name.clone())
        .collect()
}

fn confirm(yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(OlmaError::Other("non-interactive: use -y".into()));
    }
    eprint!("Proceed? [Y/n] ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(OlmaError::Io)?;
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
