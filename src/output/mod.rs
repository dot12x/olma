mod soft;

pub use soft::SoftReporter;

pub trait Reporter: Send + Sync {
    fn status(&self, msg: &str);
    fn success(&self, msg: &str);
    fn error(&self, msg: &str);
}

pub fn default_reporter() -> Box<dyn Reporter> {
    Box::new(SoftReporter::new())
}
