use std::io::{BufRead, BufReader, BufWriter, Write};

use crate::error::{AppError, AppResult};

pub struct NdjsonIo {
    stdin: BufReader<std::io::Stdin>,
    stdout: BufWriter<std::io::Stdout>,
}

impl NdjsonIo {
    pub fn new() -> Self {
        Self {
            stdin: BufReader::new(std::io::stdin()),
            stdout: BufWriter::new(std::io::stdout()),
        }
    }

    pub fn read_line(&mut self) -> AppResult<Option<String>> {
        let mut line = String::new();
        let n = self.stdin.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(&['\r', '\n'][..]).to_string();
        if line.trim().is_empty() {
            return Ok(Some(String::new()));
        }
        Ok(Some(line))
    }

    pub fn write_json_line<T: serde::Serialize>(&mut self, v: &T) -> AppResult<()> {
        serde_json::to_writer(&mut self.stdout, v)?;
        self.stdout.write_all(b"\n")?;
        self.stdout.flush()?;
        Ok(())
    }

    pub fn protocol_error(&mut self, id: String, v: u32, msg: String) -> AppResult<()> {
        #[derive(serde::Serialize)]
        struct ErrResp<'a> {
            v: u32,
            id: &'a str,
            status: &'static str,
            error: String,
            code: &'static str,
        }
        let r = ErrResp {
            v,
            id: &id,
            status: "error",
            error: msg,
            code: AppError::InvalidRequest("".into()).code(),
        };
        self.write_json_line(&r)
    }
}

