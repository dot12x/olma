use crate::error::{OlmaError, Result};
use std::path::Path;
use std::process::Command;

/// Verifies `install_name_tool` and `codesign` are available.
/// Returns `MissingXcodeCLT` if either is absent.
pub fn check_clt_available() -> Result<()> {
    for tool in &["install_name_tool", "codesign", "otool"] {
        if which(tool).is_none() {
            return Err(OlmaError::MissingXcodeCLT);
        }
    }
    Ok(())
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let cand = entry.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Reads all dependency load commands (LC_LOAD_DYLIB, LC_ID_DYLIB, LC_RPATH)
/// and any that begin with `old_prefix` are rewritten to start with `new_prefix`.
/// After modification the binary is re-signed ad-hoc.
pub fn relocate_macho(path: &Path, old_prefix: &str, new_prefix: &str) -> Result<()> {
    let load_cmds = otool_l(path)?;

    if let Some(id) = load_cmds.id_dylib.as_deref()
        && let Some(new_id) = swap_prefix(id, old_prefix, new_prefix)
    {
        run(Command::new("install_name_tool").args(["-id", &new_id, path.to_str().unwrap()]))?;
    }

    for dep in &load_cmds.load_dylibs {
        if let Some(new_dep) = swap_prefix(dep, old_prefix, new_prefix) {
            run(Command::new("install_name_tool").args([
                "-change", dep, &new_dep, path.to_str().unwrap(),
            ]))?;
        }
    }

    for rp in &load_cmds.rpaths {
        if let Some(new_rp) = swap_prefix(rp, old_prefix, new_prefix) {
            run(Command::new("install_name_tool").args([
                "-rpath", rp, &new_rp, path.to_str().unwrap(),
            ]))?;
        }
    }

    run(Command::new("codesign").args([
        "--force", "--sign", "-", path.to_str().unwrap(),
    ]))?;
    Ok(())
}

fn swap_prefix(s: &str, old: &str, new: &str) -> Option<String> {
    s.strip_prefix(old).map(|rest| format!("{new}{rest}"))
}

#[derive(Default)]
struct LoadCmds {
    id_dylib: Option<String>,
    load_dylibs: Vec<String>,
    rpaths: Vec<String>,
}

fn otool_l(path: &Path) -> Result<LoadCmds> {
    let out = Command::new("otool").args(["-l", path.to_str().unwrap()]).output()
        .map_err(|e| OlmaError::Other(format!("otool failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "otool exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut cmds = LoadCmds::default();
    let mut state = State::None;
    for line in stdout.lines() {
        let t = line.trim();
        match state {
            State::None => {
                if t.starts_with("cmd LC_ID_DYLIB") { state = State::IdDylib; }
                else if t.starts_with("cmd LC_LOAD_DYLIB") { state = State::LoadDylib; }
                else if t.starts_with("cmd LC_RPATH") { state = State::Rpath; }
            }
            State::IdDylib => {
                if let Some(rest) = t.strip_prefix("name ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.id_dylib = Some(value);
                    state = State::None;
                }
            }
            State::LoadDylib => {
                if let Some(rest) = t.strip_prefix("name ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.load_dylibs.push(value);
                    state = State::None;
                }
            }
            State::Rpath => {
                if let Some(rest) = t.strip_prefix("path ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.rpaths.push(value);
                    state = State::None;
                }
            }
        }
    }
    Ok(cmds)
}

enum State { None, IdDylib, LoadDylib, Rpath }

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().map_err(|e| OlmaError::Other(format!("spawn failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}
