mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_switches_python_family() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();

    olma::cli::add::run(
        &["python@3.11".into(), "python@3.13".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.unwrap();

    olma::cli::default::run("python@3.11", reporter.as_ref()).await.unwrap();

    let link = sandbox.bin().join("python3.11");
    let target = std::fs::read_link(&link).expect("python3.11 symlink should exist");
    let ts = target.to_string_lossy().to_string();
    assert!(ts.contains("python@3.11"), "expected python3.11 -> python@3.11, got {ts}");

    olma::cli::default::run("python@3.13", reporter.as_ref()).await.unwrap();
    let link2 = sandbox.bin().join("python3.13");
    let target2 = std::fs::read_link(&link2).unwrap();
    assert!(target2.to_string_lossy().contains("python@3.13"));
}
