use olma::config::Config;
use olma::state::{Db, packages::PackageRow, transactions::{TransactionRow, PackageChange}};

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::Builder::new().prefix("olma-state-").tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    let config = Config::from_env();
    (tmp, config)
}

#[test]
fn migration_runs_idempotently() {
    let (_t, c) = temp_config();
    let db1 = Db::open(&c).unwrap();
    drop(db1);
    let _db2 = Db::open(&c).unwrap();
}

#[test]
fn packages_round_trip() {
    let (_t, c) = temp_config();
    let db = Db::open(&c).unwrap();
    PackageRow::upsert(&db, &PackageRow {
        name: "tree".into(),
        version: "2.3.2".into(),
        installed_at: 100,
        requested: true,
        previous_ver: None,
    }).unwrap();
    let got = PackageRow::get(&db, "tree").unwrap().unwrap();
    assert_eq!(got.version, "2.3.2");
    assert!(got.requested);
}

#[test]
fn transactions_log_and_revert() {
    let (_t, c) = temp_config();
    let db = Db::open(&c).unwrap();
    let id = TransactionRow::insert(&db, "add", &[PackageChange {
        name: "tree".into(),
        from_version: None,
        to_version: Some("2.3.2".into()),
        requested: true,
    }]).unwrap();
    let latest = TransactionRow::latest_unreverted(&db).unwrap().unwrap();
    assert_eq!(latest.id, id);
    TransactionRow::mark_reverted(&db, id).unwrap();
    assert!(TransactionRow::latest_unreverted(&db).unwrap().is_none());
}
