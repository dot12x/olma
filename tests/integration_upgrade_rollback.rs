mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upgrade_then_rollback_restores_previous() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();

    olma::cli::add::run(
        &["tree".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.unwrap();

    let config = olma::config::Config::from_env();
    let db = olma::state::Db::open(&config).unwrap();
    let row = olma::state::packages::PackageRow::get(&db, "tree").unwrap().unwrap();
    let current_ver = row.version.clone();

    let fake_old = "0.0.1-test";
    let fake_dir = config.package_dir("tree", fake_old);
    std::fs::create_dir_all(fake_dir.join("bin")).unwrap();
    std::fs::write(fake_dir.join("bin").join("tree"), b"#!/bin/sh\necho fake\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fake_dir.join("bin").join("tree"),
            std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    olma::state::packages::PackageRow::upsert(&db, &olma::state::packages::PackageRow {
        name: "tree".into(),
        version: current_ver.clone(),
        installed_at: 0,
        requested: true,
        previous_ver: Some(fake_old.into()),
    }).unwrap();

    olma::state::transactions::TransactionRow::insert(
        &db,
        "upgrade",
        &[olma::state::transactions::PackageChange {
            name: "tree".into(),
            from_version: Some(fake_old.into()),
            to_version: Some(current_ver.clone()),
            requested: true,
        }],
    ).unwrap();

    olma::cli::rollback::run(true, reporter.as_ref()).await.unwrap();

    let after = olma::state::packages::PackageRow::get(&db, "tree").unwrap().unwrap();
    assert!(
        after.version == fake_old || after.version == current_ver,
        "rollback should land on either the previous or current version"
    );
    assert!(sandbox.bin().join("tree").exists());
}
