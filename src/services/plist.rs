use crate::metadata::{KeepAlive, Service};
use std::fmt::Write as _;
use std::path::Path;

pub fn render(
    name: &str,
    service: &Service,
    out_log: &Path,
    err_log: &Path,
    run_at_load: bool,
) -> String {
    let label = format!("sh.olma.{name}");
    let mut s = String::with_capacity(1024);
    s.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n",
    );

    kv_string(&mut s, "Label", &label);
    write!(
        s,
        "\t<key>RunAtLoad</key>\n\t<{}/>\n",
        if run_at_load { "true" } else { "false" }
    )
    .unwrap();

    if let Some(ka) = &service.keep_alive {
        match ka {
            KeepAlive::Bool(b) => {
                write!(
                    s,
                    "\t<key>KeepAlive</key>\n\t<{}/>\n",
                    if *b { "true" } else { "false" }
                )
                .unwrap();
            }
            KeepAlive::Obj(map) => {
                s.push_str("\t<key>KeepAlive</key>\n\t<dict>\n");
                for (k, v) in map {
                    if let Some(b) = v.as_bool() {
                        write!(
                            s,
                            "\t\t<key>{k}</key>\n\t\t<{}/>\n",
                            if b { "true" } else { "false" }
                        )
                        .unwrap();
                    } else if let Some(n) = v.as_i64() {
                        write!(s, "\t\t<key>{k}</key>\n\t\t<integer>{n}</integer>\n").unwrap();
                    }
                }
                s.push_str("\t</dict>\n");
            }
        }
    }

    if let Some(wd) = &service.working_dir {
        kv_string(&mut s, "WorkingDirectory", wd);
    }
    if let Some(pt) = &service.process_type {
        kv_string(&mut s, "ProcessType", pt);
    } else {
        kv_string(&mut s, "ProcessType", "Background");
    }
    kv_string(&mut s, "StandardOutPath", &out_log.to_string_lossy());
    kv_string(&mut s, "StandardErrorPath", &err_log.to_string_lossy());

    if !service.run.is_empty() {
        s.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        for arg in &service.run {
            write!(s, "\t\t<string>{}</string>\n", xml_escape(arg)).unwrap();
        }
        s.push_str("\t</array>\n");
    }

    if !service.environment_variables.is_empty() {
        s.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (k, v) in &service.environment_variables {
            write!(
                s,
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml_escape(k),
                xml_escape(v)
            )
            .unwrap();
        }
        s.push_str("\t</dict>\n");
    }

    s.push_str("</dict>\n</plist>\n");
    s
}

fn kv_string(buf: &mut String, key: &str, value: &str) {
    write!(
        buf,
        "\t<key>{key}</key>\n\t<string>{}</string>\n",
        xml_escape(value)
    )
    .unwrap();
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
