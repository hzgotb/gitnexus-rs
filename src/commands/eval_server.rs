use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct EvalServerOptions {
    pub port: Option<u16>,
    pub idle_timeout: Option<u32>,
}

pub fn run(options: EvalServerOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(port) = options.port {
        args.push("--port".to_string());
        args.push(port.to_string());
    }

    if let Some(idle_timeout) = options.idle_timeout {
        args.push("--idle-timeout".to_string());
        args.push(idle_timeout.to_string());
    }

    run_ts_cli("eval-server", &args)
}
