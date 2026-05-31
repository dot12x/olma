use super::Reporter;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;

const FRAMES: [&str; 10] = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];

pub struct SoftReporter {
    state: Mutex<State>,
    is_tty: bool,
}

struct State {
    frame: usize,
}

impl SoftReporter {
    pub fn new() -> Self {
        SoftReporter {
            state: Mutex::new(State { frame: 0 }),
            is_tty: std::io::stderr().is_terminal(),
        }
    }

    fn write_line(&self, prefix: &str, msg: &str) {
        let mut err = std::io::stderr().lock();
        if self.is_tty {
            // \r clears, no newline so the line is overwritten
            let _ = write!(err, "\r\x1b[2K{prefix}  {msg}");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{prefix}  {msg}");
        }
    }

    fn finish_line(&self, prefix: &str, msg: &str) {
        let mut err = std::io::stderr().lock();
        if self.is_tty {
            let _ = write!(err, "\r\x1b[2K{prefix}  {msg}\n");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{prefix}  {msg}");
        }
    }
}

impl Reporter for SoftReporter {
    fn status(&self, msg: &str) {
        let mut s = self.state.lock().unwrap();
        let frame = FRAMES[s.frame % FRAMES.len()];
        s.frame = s.frame.wrapping_add(1);
        self.write_line(frame, msg);
    }

    fn success(&self, msg: &str) {
        self.finish_line("✓", msg);
    }

    fn error(&self, msg: &str) {
        self.finish_line("✗", msg);
    }
}
