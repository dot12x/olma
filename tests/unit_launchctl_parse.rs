use olma::services::launchctl::{parse_print, ServiceState};

const RUNNING_SAMPLE: &str = r#"
gui/501/sh.olma.redis = {
    pid = 8421
    state = running
    last exit code = 0
}
"#;

const STOPPED_SAMPLE: &str = r#"
gui/501/sh.olma.redis = {
    state = not running
    last exit code = 0
}
"#;

#[test]
fn parses_running_state() {
    let snap = parse_print(RUNNING_SAMPLE);
    assert_eq!(snap.pid, Some(8421));
    assert!(matches!(snap.state, ServiceState::Running));
}

#[test]
fn parses_stopped_state() {
    let snap = parse_print(STOPPED_SAMPLE);
    assert_eq!(snap.pid, None);
    assert!(matches!(snap.state, ServiceState::Stopped));
}
