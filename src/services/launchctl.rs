use crate::error::{OlmaError, Result};
use std::path::Path;

pub fn current_uid() -> Result<u32> {
    let out = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| OlmaError::Other(format!("id -u failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other("id -u failed".into()));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|e| OlmaError::Other(format!("uid parse failed: {e}")))
}

pub fn bootstrap(plist: &Path) -> Result<()> {
    let uid = current_uid()?;
    let domain = format!("gui/{uid}");
    run(&["bootstrap", &domain, plist.to_str().unwrap()])
}

pub fn bootout(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["bootout", &target])
}

pub fn kickstart(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["kickstart", "-k", &target])
}

pub fn disable(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["disable", &target])
}

pub fn print(name: &str) -> Result<String> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    let out = std::process::Command::new("launchctl")
        .args(["print", &target])
        .output()
        .map_err(|e| OlmaError::Other(format!("launchctl print failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "launchctl print {target} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|e| OlmaError::Other(format!("launchctl spawn failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrintSnapshot {
    pub pid: Option<u32>,
    pub state: ServiceState,
    pub last_exit: Option<i32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum ServiceState {
    #[default]
    Unknown,
    Running,
    Stopped,
    Failed,
}

pub fn parse_print(out: &str) -> PrintSnapshot {
    let mut snap = PrintSnapshot::default();
    for line in out.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("pid = ") {
            snap.pid = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(rest) = t.strip_prefix("state = ") {
            snap.state = match rest.trim() {
                "running" => ServiceState::Running,
                "not running" => ServiceState::Stopped,
                _ => ServiceState::Unknown,
            };
        } else if let Some(rest) = t.strip_prefix("last exit code = ") {
            snap.last_exit = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        }
    }
    snap
}
