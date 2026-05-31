mod helpers;

use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_tree_installs_and_runs() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped: OLMA_SKIP_NETWORK_TESTS set");
        return;
    }

    let sandbox = Sandbox::new();

    let reporter = olma::output::default_reporter();
    olma::cli::add::run("tree", reporter.as_ref()).await
        .expect("add tree should succeed");

    let tree_bin = sandbox.bin().join("tree");
    assert!(tree_bin.exists(), "expected {} to exist", tree_bin.display());

    let out = std::process::Command::new(&tree_bin)
        .arg("--version")
        .output()
        .expect("tree --version should run");
    assert!(out.status.success(),
        "tree --version failed: stderr={}",
        String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tree"), "unexpected version output: {stdout}");
}
