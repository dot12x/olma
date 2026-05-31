pub mod launchctl;
pub mod plist;
pub mod store;

use crate::metadata::Service;
use crate::relocator::text::replace_prefix_in_bytes;

pub fn label_for(name: &str) -> String {
    format!("sh.olma.{name}")
}

pub fn relocate_service(service: &mut Service, swaps: &[(String, String)]) {
    for entry in service.run.iter_mut() {
        *entry = swap_all(entry, swaps);
    }
    if let Some(p) = service.working_dir.as_mut() {
        *p = swap_all(p, swaps);
    }
    if let Some(p) = service.log_path.as_mut() {
        *p = swap_all(p, swaps);
    }
    if let Some(p) = service.error_log_path.as_mut() {
        *p = swap_all(p, swaps);
    }
    for (_k, v) in service.environment_variables.iter_mut() {
        *v = swap_all(v, swaps);
    }
}

fn swap_all(s: &str, swaps: &[(String, String)]) -> String {
    let mut bytes = s.as_bytes().to_vec();
    for (old, new) in swaps {
        bytes = replace_prefix_in_bytes(&bytes, old, new);
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
}
