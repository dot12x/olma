use olma::metadata::{KeepAlive, Service};
use olma::services::plist::render;

#[test]
fn renders_minimal_plist() {
    let svc = Service {
        run: vec!["/opt/olma/packages/redis/7.4.0/bin/redis-server".to_string()],
        keep_alive: Some(KeepAlive::Bool(true)),
        working_dir: Some("/opt/olma/var".to_string()),
        log_path: None,
        error_log_path: None,
        environment_variables: Default::default(),
        process_type: None,
    };

    let xml = render(
        "redis",
        &svc,
        std::path::Path::new("/opt/olma/log/redis/out.log"),
        std::path::Path::new("/opt/olma/log/redis/err.log"),
        true,
    );

    assert!(xml.contains("<key>Label</key>\n\t<string>sh.olma.redis</string>"));
    assert!(xml.contains("<key>RunAtLoad</key>\n\t<true/>"));
    assert!(xml.contains("<key>KeepAlive</key>\n\t<true/>"));
    assert!(xml.contains("<key>WorkingDirectory</key>\n\t<string>/opt/olma/var</string>"));
    assert!(xml.contains(
        "<key>StandardOutPath</key>\n\t<string>/opt/olma/log/redis/out.log</string>"
    ));
    assert!(xml.contains("<key>ProgramArguments</key>"));
    assert!(xml.contains(
        "<string>/opt/olma/packages/redis/7.4.0/bin/redis-server</string>"
    ));
}

#[test]
fn run_at_load_off_when_not_enabled() {
    let svc = Service {
        run: vec!["/usr/bin/true".to_string()],
        keep_alive: None,
        working_dir: None,
        log_path: None,
        error_log_path: None,
        environment_variables: Default::default(),
        process_type: None,
    };
    let xml = render(
        "noop",
        &svc,
        std::path::Path::new("/tmp/out"),
        std::path::Path::new("/tmp/err"),
        false,
    );
    assert!(xml.contains("<key>RunAtLoad</key>\n\t<false/>"));
}
