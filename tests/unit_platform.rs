use olma::platform::{Arch, macos_codename_for_product_version};

#[test]
fn arch_current_returns_known_value() {
    let a = Arch::current();
    assert!(matches!(a, Arch::Arm64 | Arch::X64));
}

#[test]
fn arch_as_bottle_str_returns_homebrew_names() {
    assert_eq!(Arch::Arm64.as_bottle_str(), "arm64");
    assert_eq!(Arch::X64.as_bottle_str(), "x86_64");
}

#[test]
fn codename_known_versions() {
    assert_eq!(macos_codename_for_product_version("13.0"), Some("ventura"));
    assert_eq!(macos_codename_for_product_version("13.6.1"), Some("ventura"));
    assert_eq!(macos_codename_for_product_version("14.5"), Some("sonoma"));
    assert_eq!(macos_codename_for_product_version("15.0"), Some("sequoia"));
    assert_eq!(macos_codename_for_product_version("26.0"), Some("tahoe"));
}

#[test]
fn codename_unknown_returns_none() {
    assert_eq!(macos_codename_for_product_version("99.0"), None);
    assert_eq!(macos_codename_for_product_version("not a version"), None);
}
