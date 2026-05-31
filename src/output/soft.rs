use super::Reporter;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const C_SPIN: &str = "\x1b[36m";
const C_OK:   &str = "\x1b[32m";
const C_ERR:  &str = "\x1b[31m";
const C_DIM:  &str = "\x1b[2m";
const C_OFF:  &str = "\x1b[0m";

pub struct SoftReporter {
    state: Arc<Mutex<State>>,
    alive: Arc<AtomicBool>,
}

struct State {
    msg: Option<String>,
    frame: usize,
    is_tty: bool,
}

impl Default for SoftReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftReporter {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(State {
            msg: None,
            frame: 0,
            is_tty: std::io::stderr().is_terminal(),
        }));
        let alive = Arc::new(AtomicBool::new(true));
        let s = state.clone();
        let a = alive.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(80));
            interval.tick().await;
            while a.load(Ordering::Relaxed) {
                interval.tick().await;
                let snap = {
                    let mut st = s.lock().unwrap();
                    if st.msg.is_some() {
                        let frame = FRAMES[st.frame % FRAMES.len()];
                        st.frame = st.frame.wrapping_add(1);
                        let msg = st.msg.as_ref().unwrap().clone();
                        Some((frame, msg, st.is_tty))
                    } else {
                        None
                    }
                };
                if let Some((frame, msg, is_tty)) = snap
                    && is_tty
                {
                    let mut err = std::io::stderr().lock();
                    let _ = write!(err, "\r\x1b[2K{C_SPIN}{frame}{C_OFF}  {msg}");
                    let _ = err.flush();
                }
            }
        });
        Self { state, alive }
    }
}

impl Drop for SoftReporter {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl Reporter for SoftReporter {
    fn status(&self, msg: &str) {
        let (is_tty, frame) = {
            let mut st = self.state.lock().unwrap();
            st.msg = Some(msg.to_string());
            st.frame = 0;
            (st.is_tty, FRAMES[0])
        };
        let mut err = std::io::stderr().lock();
        if is_tty {
            let _ = write!(err, "\r\x1b[2K{C_SPIN}{frame}{C_OFF}  {msg}");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{C_DIM}·{C_OFF}  {msg}");
        }
    }

    fn success(&self, msg: &str) {
        let is_tty = {
            let mut st = self.state.lock().unwrap();
            st.msg = None;
            st.is_tty
        };
        let mut err = std::io::stderr().lock();
        if is_tty {
            let _ = write!(err, "\r\x1b[2K{C_OK}✓{C_OFF}  {msg}\n");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{C_OK}✓{C_OFF}  {msg}");
        }
    }

    fn error(&self, msg: &str) {
        let is_tty = {
            let mut st = self.state.lock().unwrap();
            st.msg = None;
            st.is_tty
        };
        let mut err = std::io::stderr().lock();
        if is_tty {
            let _ = write!(err, "\r\x1b[2K{C_ERR}✗{C_OFF}  {msg}\n");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{C_ERR}✗{C_OFF}  {msg}");
        }
    }
}
