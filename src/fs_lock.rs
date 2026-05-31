use crate::error::{OlmaError, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

pub struct WriteLock {
    file: File,
}

impl WriteLock {
    /// Try to acquire the lock immediately. Returns `Err(LockBusy)` if held.
    pub fn try_acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(WriteLock { file }),
            Err(_) => Err(OlmaError::LockBusy(path.to_path_buf())),
        }
    }

    /// Acquire the lock, polling every second. Cancellable by SIGINT (caller responsibility).
    pub async fn acquire_blocking(path: &Path) -> Result<Self> {
        loop {
            match Self::try_acquire(path) {
                Ok(lock) => return Ok(lock),
                Err(OlmaError::LockBusy(_)) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
