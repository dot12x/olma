#![allow(dead_code)]

use std::path::PathBuf;

/// A throwaway OLMA_ROOT under tempdir.
pub struct Sandbox {
    pub root: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let root = tempfile::Builder::new().prefix("olma-test-").tempdir().unwrap();
        // SAFETY: tests are single-threaded with respect to env vars when run sequentially;
        // each Sandbox instance owns a unique tempdir.
        unsafe { std::env::set_var("OLMA_ROOT", root.path()); }
        Sandbox { root }
    }

    pub fn root_path(&self) -> PathBuf {
        self.root.path().to_path_buf()
    }

    pub fn bin(&self) -> PathBuf { self.root_path().join("bin") }
    pub fn packages(&self) -> PathBuf { self.root_path().join("packages") }
}
