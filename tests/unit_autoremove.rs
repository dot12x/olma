use olma::cli::autoremove::classify_orphans;
use olma::state::packages::PackageRow;

fn row(name: &str, requested: bool) -> PackageRow {
    PackageRow {
        name: name.into(),
        version: "1.0".into(),
        installed_at: 0,
        requested,
        previous_ver: None,
    }
}

#[test]
fn orphan_when_no_requested_consumer() {
    let rows = vec![
        row("tree", true),
        row("orphan-lib", false),
    ];
    let orphans = classify_orphans(&rows, &|_| vec![]);
    assert_eq!(orphans, vec!["orphan-lib".to_string()]);
}

#[test]
fn not_orphan_when_referenced() {
    let rows = vec![
        row("jq", true),
        row("oniguruma", false),
    ];
    let orphans = classify_orphans(&rows, &|n| {
        if n == "jq" { vec!["oniguruma".into()] } else { vec![] }
    });
    assert!(orphans.is_empty());
}

#[test]
fn requested_packages_never_orphan() {
    let rows = vec![row("tree", true)];
    assert!(classify_orphans(&rows, &|_| vec![]).is_empty());
}
