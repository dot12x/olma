use crate::cache::FormulaCache;
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::Formula;
use reqwest::header::{HeaderMap, HeaderValue, ETAG, IF_NONE_MATCH};
use reqwest::StatusCode;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPolicy {
    CacheFirst,
    ForceRefresh,
    BypassCache,
    OfflineOnly,
}

pub struct FormulaeClient {
    http: reqwest::Client,
    base: String,
    config: Config,
}

impl FormulaeClient {
    pub fn new(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self {
            http,
            base: config.formulae_api_base().to_string(),
            config: config.clone(),
        })
    }

    pub async fn fetch(&self, name: &str, policy: FetchPolicy) -> Result<Formula> {
        let cache = FormulaCache::new(&self.config);

        if policy == FetchPolicy::OfflineOnly {
            let entry = cache.read(name)?
                .ok_or_else(|| OlmaError::Network(format!("offline: no cached metadata for {name}")))?;
            return serde_json::from_str(&entry.json)
                .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")));
        }

        let cached = if policy == FetchPolicy::BypassCache {
            None
        } else {
            cache.read(name)?
        };

        if policy == FetchPolicy::CacheFirst
            && let Some(entry) = &cached
        {
            return serde_json::from_str(&entry.json)
                .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")));
        }

        let url = format!("{}/{}.json", self.base, name);
        let mut headers = HeaderMap::new();
        if policy == FetchPolicy::ForceRefresh
            && let Some(entry) = &cached
            && let Some(etag) = &entry.etag
            && let Ok(v) = HeaderValue::from_str(etag)
        {
            headers.insert(IF_NONE_MATCH, v);
        }

        let resp = self.http.get(&url).headers(headers).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;

        match resp.status() {
            StatusCode::NOT_MODIFIED => {
                let entry = cached.ok_or_else(|| OlmaError::Network("304 without cache".into()))?;
                serde_json::from_str(&entry.json)
                    .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")))
            }
            StatusCode::OK => {
                let etag = resp.headers().get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let body = resp.text().await
                    .map_err(|e| OlmaError::Network(e.to_string()))?;
                if policy != FetchPolicy::BypassCache {
                    cache.write(name, &body, etag.as_deref())?;
                }
                serde_json::from_str(&body)
                    .map_err(|e| OlmaError::Network(format!("formula JSON parse failed: {e}")))
            }
            StatusCode::NOT_FOUND => Err(OlmaError::FormulaNotFound(name.to_string())),
            s => Err(OlmaError::Network(format!("formulae.brew.sh returned {s}"))),
        }
    }
}
