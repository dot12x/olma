pub mod download;
pub mod extract;
pub mod link;
pub mod relocate;
pub mod verify;

use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::ghcr::GhcrClient;
use crate::output::Reporter;
use crate::resolver::InstallPlan;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

pub struct Pipeline {
    config: Arc<Config>,
    bottle_tag: String,
    reporter: Arc<dyn Reporter>,
}

impl Pipeline {
    pub fn new(config: Arc<Config>, bottle_tag: String, reporter: Arc<dyn Reporter>) -> Self {
        Self { config, bottle_tag, reporter }
    }

    pub async fn run(&self, plan: InstallPlan) -> Result<()> {
        let download_n = std::cmp::min(8, num_cpus::get() * 2);
        let cpu_n = num_cpus::get();
        let dl_sem = Arc::new(Semaphore::new(download_n));
        let cpu_sem = Arc::new(Semaphore::new(cpu_n));
        let ghcr = Arc::new(GhcrClient::new()?);

        let total = plan.ordered.len();
        self.reporter.status(&format!("Resolving {total} packages"));

        let (dl_tx, mut dl_rx) = mpsc::channel::<download::DownloadedWithFormula>(64);

        let mut download_handles = Vec::new();
        for formula in plan.ordered.iter().cloned() {
            let tag = self.bottle_tag.clone();
            let config = self.config.clone();
            let ghcr = ghcr.clone();
            let dl_sem = dl_sem.clone();
            let dl_tx = dl_tx.clone();
            let reporter = self.reporter.clone();
            download_handles.push(tokio::spawn(async move {
                let _permit = dl_sem.acquire_owned().await
                    .map_err(|e| OlmaError::Other(format!("dl sem: {e}")))?;
                let bottle = formula.bottle_for_tag(&tag)
                    .ok_or_else(|| OlmaError::BottleNotForPlatform {
                        name: formula.name.clone(),
                        platform: tag.clone(),
                    })?;
                reporter.status(&format!("Downloading {} {}", formula.name, formula.version()));
                let dl = download::download(&ghcr, bottle, &config).await?;
                verify::verify_sha256(&bottle.sha256, &dl.sha256_hex)?;
                dl_tx.send(download::DownloadedWithFormula::new(dl, formula))
                    .await
                    .map_err(|e| OlmaError::Other(format!("dl chan send: {e}")))?;
                Ok::<(), OlmaError>(())
            }));
        }
        drop(dl_tx);

        let mut staged = Vec::new();
        while let Some(item) = dl_rx.recv().await {
            let cpu_sem = cpu_sem.clone();
            let config = self.config.clone();
            let reporter = self.reporter.clone();
            staged.push(tokio::spawn(async move {
                let _permit = cpu_sem.acquire_owned().await
                    .map_err(|e| OlmaError::Other(format!("cpu sem: {e}")))?;
                reporter.status(&format!("Relocating {}", item.formula.name));
                extract::extract_and_relocate(&item, &config).await?;
                Ok::<download::DownloadedWithFormula, OlmaError>(item)
            }));
        }

        let mut linked: Vec<download::DownloadedWithFormula> = Vec::new();
        for h in staged {
            let item = h.await
                .map_err(|e| OlmaError::Other(format!("stage task: {e}")))??;
            linked.push(item);
        }

        for h in download_handles {
            h.await.map_err(|e| OlmaError::Other(format!("dl task: {e}")))??;
        }

        let _lock = WriteLock::acquire_blocking(&self.config.lock_file()).await?;
        for item in &linked {
            let dest = self.config.package_dir(&item.formula.name, item.formula.version());
            link::link_bin(&self.config, &dest)?;
        }

        let requested: Vec<&str> = plan.requested.iter().map(|s| s.as_str()).collect();
        self.reporter.success(&format!("Installed {} packages ({})", linked.len(), requested.join(", ")));
        Ok(())
    }
}
