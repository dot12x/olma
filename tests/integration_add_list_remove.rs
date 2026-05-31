mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_lifecycle() {
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

    assert!(sandbox.bin().join("tree").exists());

    let config = olma::config::Config::from_env();
    let db = olma::state::Db::open(&config).unwrap();
    let rows = olma::state::packages::PackageRow::list(&db).unwrap();
    assert_eq!(rows.iter().filter(|r| r.name == "tree").count(), 1);

    olma::cli::remove::run(
        &["tree".into()],
        true,
        reporter.as_ref(),
    ).await.unwrap();

    assert!(!sandbox.bin().join("tree").exists());
    let rows_after = olma::state::packages::PackageRow::list(&db).unwrap();
    assert!(rows_after.iter().all(|r| r.name != "tree"));

    olma::cli::rollback::run(true, reporter.as_ref()).await.unwrap();
    assert!(sandbox.bin().join("tree").exists());
}
