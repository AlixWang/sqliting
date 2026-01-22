use tracing_subscriber::EnvFilter;

pub fn init(log_level: &str) {
    // Prefer explicit --log-level; allow RUST_LOG override.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

