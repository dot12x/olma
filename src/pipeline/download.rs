use crate::config::Paths;
use crate::error::{OlmaError, Result};
use crate::metadata::{BottleFile, ghcr::GhcrClient};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWrite;

pub struct Downloaded {
    /// Final path of the cached bottle.
    pub path: PathBuf,
    /// Hex-encoded sha256 of the bytes written.
    pub sha256_hex: String,
    pub bytes: u64,
}

/// Downloads the bottle to `cache/bottles/<expected_sha256>.tar.gz`.
/// Returns the computed sha256 so the caller can verify against the formula's
/// declared value.
pub async fn download(
    ghcr: &GhcrClient,
    bottle: &BottleFile,
    paths: &Paths,
) -> Result<Downloaded> {
    std::fs::create_dir_all(paths.cache_bottles())?;

    let final_path = paths.cache_bottles().join(format!("{}.tar.gz", bottle.sha256));
    if final_path.exists() {
        // Reuse cached, but rehash to confirm integrity.
        let bytes = tokio::fs::read(&final_path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hex = hex::encode(hasher.finalize());
        return Ok(Downloaded {
            path: final_path,
            sha256_hex: hex,
            bytes: bytes.len() as u64,
        });
    }

    // Homebrew bottle URLs look like:
    //   https://ghcr.io/v2/homebrew/core/<name>/blobs/sha256:<digest>
    let scope_repo = extract_scope_repo(&bottle.url)
        .ok_or_else(|| OlmaError::Network(format!("unexpected bottle url: {}", bottle.url)))?;
    let token = ghcr.token(&scope_repo).await?;

    let partial = paths.cache_bottles().join(format!("{}.tar.gz.partial", bottle.sha256));
    let f = File::create(&partial).await?;
    let mut hashing = HashingWriter::new(f);
    let bytes = ghcr.fetch_blob(&bottle.url, &token, &mut hashing).await?;
    let hex = hashing.finish_hex();

    tokio::fs::rename(&partial, &final_path).await?;
    Ok(Downloaded { path: final_path, sha256_hex: hex, bytes })
}

fn extract_scope_repo(url: &str) -> Option<String> {
    // Expect: https://ghcr.io/v2/<owner>/<repo>/blobs/sha256:<digest>
    let path = url.split("ghcr.io/").nth(1)?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 6 { return None; }
    Some(format!("{}/{}/{}", parts[1], parts[2], parts[3]))
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: AsyncWrite + Unpin> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, hasher: Sha256::new() }
    }
    fn finish_hex(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for HashingWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let res = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &res {
            self.hasher.update(&buf[..*n]);
        }
        res
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
