use crate::error::{OlmaError, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct TokenResp {
    token: String,
}

pub struct GhcrClient {
    http: reqwest::Client,
}

impl GhcrClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    /// Returns an anonymous pull token for a single repository scope, e.g. `homebrew/core/tree`.
    pub async fn token(&self, scope_repo: &str) -> Result<String> {
        let url = format!(
            "https://ghcr.io/token?service=ghcr.io&scope=repository:{scope_repo}:pull"
        );
        let resp = self.http.get(&url).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(OlmaError::Network(format!("ghcr.io token request failed: {}", resp.status())));
        }
        let body: TokenResp = resp.json().await
            .map_err(|e| OlmaError::Network(format!("invalid token response: {e}")))?;
        Ok(body.token)
    }

    /// Streams the blob bytes into `sink`; returns total bytes written.
    pub async fn fetch_blob<W>(
        &self,
        url: &str,
        token: &str,
        mut sink: W,
    ) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        use tokio::io::AsyncWriteExt;
        use futures_util::StreamExt;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
        headers.insert("Accept", HeaderValue::from_static("application/vnd.oci.image.layer.v1.tar+gzip"));

        let resp = self.http.get(url).headers(headers).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(OlmaError::Network(format!("ghcr blob fetch failed: {}", resp.status())));
        }
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| OlmaError::Network(e.to_string()))?;
            sink.write_all(&bytes).await?;
            total += bytes.len() as u64;
        }
        sink.flush().await?;
        Ok(total)
    }
}
