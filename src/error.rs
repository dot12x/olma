use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, thiserror::Error)]
pub enum OlmaError {
    #[error("package \"{0}\" not found")]
    FormulaNotFound(String),

    #[error("no macOS bottle for {name} on {platform}")]
    BottleNotForPlatform { name: String, platform: String },

    #[error("network error: {0}")]
    Network(String),

    #[error("bottle checksum mismatch — possible corruption or tampering")]
    ChecksumMismatch,

    #[error("cannot write {0}. Run installer or check permissions.")]
    RootWriteDenied(PathBuf),

    #[error("olma requires Xcode Command Line Tools. Run: xcode-select --install")]
    MissingXcodeCLT,

    #[error("{0} is locked by another olma process")]
    LockBusy(PathBuf),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl OlmaError {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            OlmaError::FormulaNotFound(_) => ExitCode::from(65),
            OlmaError::BottleNotForPlatform { .. } => ExitCode::from(66),
            OlmaError::Network(_) => ExitCode::from(74),
            OlmaError::ChecksumMismatch => ExitCode::from(1),
            OlmaError::RootWriteDenied(_) => ExitCode::from(78),
            OlmaError::MissingXcodeCLT => ExitCode::from(78),
            OlmaError::LockBusy(_) => ExitCode::from(1),
            OlmaError::Io(_) => ExitCode::from(74),
            OlmaError::Other(_) => ExitCode::from(1),
        }
    }
}

pub type Result<T> = std::result::Result<T, OlmaError>;
