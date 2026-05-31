use crate::error::{OlmaError, Result};

pub fn verify_sha256(expected: &str, actual: &str) -> Result<()> {
    if expected.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        Err(OlmaError::ChecksumMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive() {
        assert!(verify_sha256("ABC123", "abc123").is_ok());
    }

    #[test]
    fn mismatch_returns_error() {
        assert!(matches!(
            verify_sha256("abc", "def"),
            Err(OlmaError::ChecksumMismatch)
        ));
    }
}
