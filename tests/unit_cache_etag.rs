use olma::cache::FormulaCache;
use olma::config::Config;

#[test]
fn etag_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    let config = Config::from_env();
    let cache = FormulaCache::new(&config);

    let body = r#"{"name":"x","versions":{"stable":"1.0"},"bottle":{"stable":{"rebuild":null,"root_url":"r","files":{}}}}"#;
    cache.write("x", body, Some("\"abc123\"")).unwrap();

    let entry = cache.read("x").unwrap().unwrap();
    assert_eq!(entry.json, body);
    assert_eq!(entry.etag.as_deref(), Some("\"abc123\""));
}
