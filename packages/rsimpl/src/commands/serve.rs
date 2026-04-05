use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    pub port: Option<u16>,
    pub host: Option<String>,
}

pub fn run(options: ServeOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(port) = options.port {
        args.push("--port".to_string());
        args.push(port.to_string());
    }

    if let Some(host) = options.host {
        args.push("--host".to_string());
        args.push(host);
    }

    run_ts_cli("serve", &args)
}
