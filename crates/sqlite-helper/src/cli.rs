use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "sqlite-helper")]
pub struct Args {
    /// Run as MCP server (JSON-RPC 2.0 over stdio)
    #[arg(long)]
    pub mcp: bool,

    /// Logging level (stderr). Also supports RUST_LOG.
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Maximum rows returned per query (unless a smaller limit is provided).
    #[arg(long, default_value_t = 1000)]
    pub max_rows: usize,

    /// Soft timeout for a single request.
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,

    /// Allowed directory whitelist (repeatable). Mainly for MCP mode.
    #[arg(long)]
    pub allowed_dir: Vec<PathBuf>,

    /// Force protocol version (reserved for future).
    #[arg(long)]
    pub protocol_version: Option<u32>,
}

