use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::{client::FormulaeClient, ghcr::GhcrClient};
use crate::output::Reporter;
use crate::pipeline::{download, extract, link, relocate, verify};
use crate::platform::current_bottle_tag;
use crate::relocator::macho;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    macho::check_clt_available()?;

    let config = Config::from_env();
    config.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;

    reporter.status(&format!("Fetching {name} metadata"));
    let client = FormulaeClient::new(&config)?;
    let formula = client.fetch(name).await?;

    if !formula.dependencies.is_empty() {
        return Err(OlmaError::Other(format!(
            "{name} has dependencies; MVP supports zero-dep formulas only. \
             Try `tree`, `jq`, or `fd`."
        )));
    }

    let tag = current_bottle_tag().await?;
    let bottle = formula.bottle_for_tag(&tag)
        .ok_or_else(|| OlmaError::BottleNotForPlatform {
            name: name.to_string(),
            platform: tag.clone(),
        })?;

    reporter.status(&format!("Downloading {name} {}", formula.version()));
    let ghcr = GhcrClient::new()?;
    let dl = download::download(&ghcr, bottle, &config).await?;
    verify::verify_sha256(&bottle.sha256, &dl.sha256_hex)?;

    reporter.status(&format!("Extracting {name}"));
    let staging = config.cache().join("staging")
        .join(format!("{}-{}", formula.name, formula.version()));
    if staging.exists() { std::fs::remove_dir_all(&staging)?; }
    std::fs::create_dir_all(&staging)?;
    extract::extract(&dl.path, &staging).await?;
    let inner = staging.join(&formula.name).join(formula.version());
    if !inner.is_dir() {
        return Err(OlmaError::Other(format!(
            "unexpected bottle layout: expected {}", inner.display()
        )));
    }
    let dest = config.package_dir(&formula.name, formula.version());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&staging);

    reporter.status(&format!("Relocating {name}"));
    let new_cellar = dest.to_string_lossy().to_string();
    let new_root = config.root.to_string_lossy().to_string();
    let swaps: Vec<(String, String)> = vec![
        (format!("/opt/homebrew/Cellar/{}/{}", formula.name, formula.version()), new_cellar.clone()),
        (format!("/usr/local/Cellar/{}/{}", formula.name, formula.version()), new_cellar.clone()),
        ("/opt/homebrew".to_string(), new_root.clone()),
        ("/usr/local".to_string(), new_root.clone()),
    ];
    relocate::relocate_tree(&dest, swaps).await?;

    reporter.status(&format!("Linking {name} into bin/"));
    let stats = link::link_bin(&config, &dest)?;
    for collision in &stats.skipped_collisions {
        reporter.error(&format!("symlink collision: {collision} (use --force-link to override)"));
    }

    reporter.success(&format!(
        "Installed {} {} ({} bytes)",
        formula.name, formula.version(), dl.bytes
    ));
    Ok(())
}
