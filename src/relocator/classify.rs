#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    MachO,
    Text,
    Binary,
}

const MACHO_MAGICS: [[u8; 4]; 4] = [
    [0xFE, 0xED, 0xFA, 0xCE],
    [0xFE, 0xED, 0xFA, 0xCF],
    [0xCE, 0xFA, 0xED, 0xFE],
    [0xCF, 0xFA, 0xED, 0xFE],
];

const FAT_MAGICS: [[u8; 4]; 4] = [
    [0xCA, 0xFE, 0xBA, 0xBE],
    [0xBE, 0xBA, 0xFE, 0xCA],
    [0xCA, 0xFE, 0xBA, 0xBF],
    [0xBF, 0xBA, 0xFE, 0xCA],
];

pub fn classify_bytes(head: &[u8]) -> FileKind {
    if head.len() >= 4 {
        let m = [head[0], head[1], head[2], head[3]];
        if MACHO_MAGICS.contains(&m) || FAT_MAGICS.contains(&m) {
            return FileKind::MachO;
        }
    }
    let probe_len = head.len().min(512);
    let probe = &head[..probe_len];
    if probe.contains(&0) {
        return FileKind::Binary;
    }
    if std::str::from_utf8(probe).is_ok() {
        FileKind::Text
    } else {
        FileKind::Binary
    }
}

/// Reads up to `len` bytes from `path` and classifies.
pub fn classify_path(path: &std::path::Path) -> std::io::Result<FileKind> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 512];
    let n = f.read(&mut buf)?;
    Ok(classify_bytes(&buf[..n]))
}
