use olma::cli::default::family_of;

#[test]
fn family_of_unversioned() {
    assert_eq!(family_of("python"), "python");
    assert_eq!(family_of("tree"), "tree");
}

#[test]
fn family_of_versioned() {
    assert_eq!(family_of("python@3.11"), "python");
    assert_eq!(family_of("python@3.13"), "python");
    assert_eq!(family_of("node@18"), "node");
}

#[test]
fn family_of_double_at_keeps_first() {
    assert_eq!(family_of("weird@1@2"), "weird");
}
