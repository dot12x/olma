use crate::config::formulae_api_base;
use crate::error::{OlmaError, Result};
use crate::metadata::Formula;
use reqwest::StatusCode;
use std::time::Duration;

pub struct FormulaeClient {
    http: reqwest::Client,
}

impl FormulaeClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    pub async fn fetch(&self, name: &str) -> Result<Formula> {
        let url = format!("{}/{}.json", formulae_api_base(), name);
        let resp = self.http.get(&url).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(OlmaError::FormulaNotFound(name.to_string())),
            s => return Err(OlmaError::Network(format!("formulae.brew.sh returned {s}"))),
        }
        let formula: Formula = resp.json().await
            .map_err(|e| OlmaError::Network(format!("failed to parse formula JSON: {e}")))?;
        Ok(formula)
    }
}
