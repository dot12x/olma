use crate::config::Config;
use crate::error::{OlmaError, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    #[serde(default)]
    pub desc: Option<String>,
}

pub struct Index {
    pub entries: Vec<IndexEntry>,
}

impl Index {
    pub fn load(config: &Config) -> Result<Option<Self>> {
        let path = config.cache_formulae().join("__index.json");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let entries: Vec<IndexEntry> = serde_json::from_slice(&bytes)
            .map_err(|e| OlmaError::Other(format!("__index.json parse: {e}")))?;
        Ok(Some(Self { entries }))
    }

    pub fn substring(&self, query: &str) -> Vec<&IndexEntry> {
        let q = query.to_lowercase();
        self.entries.iter()
            .filter(|e| e.name.to_lowercase().contains(&q)
                || e.desc.as_deref().map(|d| d.to_lowercase().contains(&q)).unwrap_or(false))
            .collect()
    }

    pub fn did_you_mean(&self, query: &str, n: usize) -> Vec<&IndexEntry> {
        let mut scored: Vec<(usize, &IndexEntry)> = self.entries.iter()
            .map(|e| (levenshtein(&e.name, query), e))
            .collect();
        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(n).map(|(_, e)| e).collect()
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1)
                .min(prev[j] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}
