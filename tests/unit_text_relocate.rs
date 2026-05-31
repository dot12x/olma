use olma::relocator::text::replace_prefix_in_bytes;

#[test]
fn replaces_single_occurrence() {
    let input = b"prefix /opt/homebrew/Cellar/foo and more".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, b"prefix /opt/olma/Cellar/foo and more");
}

#[test]
fn replaces_multiple_occurrences() {
    let input = b"a /opt/homebrew b /opt/homebrew c".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, b"a /opt/olma b /opt/olma c");
}

#[test]
fn no_match_returns_original() {
    let input = b"nothing to replace here".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, input);
}

#[test]
fn replacement_with_different_lengths() {
    let input = b"/usr/local/bin/foo".to_vec();
    let out = replace_prefix_in_bytes(&input, "/usr/local", "/opt/olma");
    assert_eq!(out, b"/opt/olma/bin/foo");
}
