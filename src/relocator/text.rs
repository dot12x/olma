use memchr::memmem;

pub fn replace_prefix_in_bytes(input: &[u8], old: &str, new: &str) -> Vec<u8> {
    let old_b = old.as_bytes();
    let new_b = new.as_bytes();
    let finder = memmem::Finder::new(old_b);
    let positions: Vec<usize> = finder.find_iter(input).collect();
    if positions.is_empty() {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len() + positions.len() * new_b.len().saturating_sub(old_b.len()));
    let mut cursor = 0;
    for pos in positions {
        out.extend_from_slice(&input[cursor..pos]);
        out.extend_from_slice(new_b);
        cursor = pos + old_b.len();
    }
    out.extend_from_slice(&input[cursor..]);
    out
}

/// Reads the file, replaces, and writes back if any change occurred.
/// Returns Ok(true) if the file was rewritten.
pub fn relocate_text_file(
    path: &std::path::Path,
    old_prefix: &str,
    new_prefix: &str,
) -> std::io::Result<bool> {
    let bytes = std::fs::read(path)?;
    let replaced = replace_prefix_in_bytes(&bytes, old_prefix, new_prefix);
    if replaced == bytes {
        return Ok(false);
    }
    let meta = std::fs::metadata(path)?;
    let tmp = path.with_extension("relocate.tmp");
    std::fs::write(&tmp, &replaced)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(meta.permissions().mode());
        std::fs::set_permissions(&tmp, perms)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(true)
}
