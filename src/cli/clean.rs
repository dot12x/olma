use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let bottles = config.cache_bottles();
    if !bottles.exists() {
        reporter.success("Cache already empty");
        return Ok(());
    }

    let (count, total_bytes) = tally(&bottles)?;
    if count == 0 {
        reporter.success("Cache already empty");
        return Ok(());
    }
    reporter.status(&format!("Will delete {count} cached bottles ({})", human(total_bytes)));
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }
    for entry in std::fs::read_dir(&bottles)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("gz") {
            let _ = std::fs::remove_file(&path);
        }
    }
    reporter.success(&format!("Cleared {count} bottles ({})", human(total_bytes)));
    Ok(())
}

fn tally(dir: &std::path::Path) -> Result<(usize, u64)> {
    let mut c = 0usize;
    let mut b = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            c += 1;
            b += meta.len();
        }
    }
    Ok((c, b))
}

fn human(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
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
