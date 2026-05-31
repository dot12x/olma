pub mod client;
pub mod ghcr;

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Formula {
    pub name: String,
    pub desc: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub versions: Versions,
    pub bottle: BottleSpec,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Versions {
    pub stable: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BottleSpec {
    pub stable: BottleStable,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BottleStable {
    pub rebuild: Option<u32>,
    pub root_url: String,
    pub files: HashMap<String, BottleFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BottleFile {
    pub cellar: String,
    pub url: String,
    pub sha256: String,
}

impl Formula {
    pub fn bottle_for_tag(&self, tag: &str) -> Option<&BottleFile> {
        self.bottle.stable.files.get(tag)
            .or_else(|| self.bottle.stable.files.get("all"))
    }

    pub fn version(&self) -> &str {
        &self.versions.stable
    }
}
