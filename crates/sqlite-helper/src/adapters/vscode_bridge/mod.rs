mod handler;
mod io;
mod protocol;

use crate::{cli::Args, error::AppResult};

use handler::BridgeHandler;
use io::NdjsonIo;
use protocol::BridgeRequest;

pub fn run(args: Args) -> AppResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;

    rt.block_on(async move {
        let mut io = NdjsonIo::new();
        let mut handler = BridgeHandler::new(args);

        loop {
            let Some(line) = io.read_line()? else { break };
            if line.is_empty() {
                continue;
            }

            let req: BridgeRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    // best-effort: unknown id; still return something
                    let _ = io.protocol_error("".to_string(), 1, e.to_string());
                    continue;
                }
            };

            let resp = handler.handle(req).await;
            io.write_json_line(&resp)?;
        }

        Ok(())
    })
}

