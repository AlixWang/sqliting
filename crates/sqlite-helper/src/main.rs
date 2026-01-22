mod adapters;
mod cli;
mod core;
mod error;
mod logging;

use clap::Parser;

use crate::{cli::Args, error::AppResult};

fn main() -> AppResult<()> {
    let args = Args::parse();
    logging::init(&args.log_level);

    if args.mcp {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| error::AppError::Internal(e.to_string()))?;
        rt.block_on(adapters::mcp::server::run(args))
    } else {
        adapters::vscode_bridge::run(args)
    }
}

