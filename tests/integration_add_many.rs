mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_three_packages_in_parallel() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();
    olma::cli::add::run(
        &["tree".into(), "jq".into(), "fd".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.expect("add three");

    for exe in ["tree", "jq", "fd"] {
        let p = sandbox.bin().join(exe);
        assert!(p.exists(), "expected {} linked", exe);
        let out = std::process::Command::new(&p).arg("--version").output().unwrap();
        assert!(out.status.success(), "{} --version failed", exe);
    }
}
