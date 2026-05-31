mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_ripgrep_pulls_pcre2() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }

    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();
    olma::cli::add::run(
        &["ripgrep".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.expect("add ripgrep");

    let rg = sandbox.bin().join("rg");
    assert!(rg.exists(), "rg should be linked");

    let out = std::process::Command::new(&rg).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ripgrep"), "got: {stdout}");
}
