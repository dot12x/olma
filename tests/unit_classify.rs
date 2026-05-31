use olma::relocator::classify::{classify_bytes, FileKind};

#[test]
fn macho_64bit_magic_recognized() {
    let bytes = [0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0];
    assert_eq!(classify_bytes(&bytes), FileKind::MachO);
}

#[test]
fn macho_universal_magic_recognized() {
    let bytes = [0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 0];
    assert_eq!(classify_bytes(&bytes), FileKind::MachO);
}

#[test]
fn utf8_text_recognized() {
    let bytes = b"# Some text\nhello\n";
    assert_eq!(classify_bytes(bytes), FileKind::Text);
}

#[test]
fn null_bytes_classified_as_binary() {
    let bytes = [0u8, 1, 2, 3, 4, 5, 6, 7];
    assert_eq!(classify_bytes(&bytes), FileKind::Binary);
}

#[test]
fn empty_is_text() {
    assert_eq!(classify_bytes(b""), FileKind::Text);
}
