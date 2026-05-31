use crate::error::{OlmaError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X64,
}

impl Arch {
    pub fn current() -> Self {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }

    pub fn as_bottle_str(&self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X64 => "x86_64",
        }
    }
}

pub fn macos_codename_for_product_version(product_version: &str) -> Option<&'static str> {
    let major: u32 = product_version.split('.').next()?.parse().ok()?;
    match major {
        11 => Some("bigsur"),
        12 => Some("monterey"),
        13 => Some("ventura"),
        14 => Some("sonoma"),
        15 => Some("sequoia"),
        26 => Some("tahoe"),
        _ => None,
    }
}

pub async fn detect_macos_codename() -> Result<String> {
    let out = tokio::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .await
        .map_err(|e| OlmaError::Other(format!("failed to run sw_vers: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other("sw_vers failed".into()));
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    macos_codename_for_product_version(&version)
        .map(|s| s.to_string())
        .ok_or_else(|| OlmaError::Other(format!("unsupported macOS version: {version}")))
}

/// Returns the Homebrew bottle tag for this host, e.g. "arm64_sonoma".
pub async fn current_bottle_tag() -> Result<String> {
    Ok(format!("{}_{}", Arch::current().as_bottle_str(), detect_macos_codename().await?))
}
