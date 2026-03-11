use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

pub fn run(pattern: &str) -> Result<()> {
    let args = vec![pattern.to_string()];
    run_ts_cli("augment", &args)
}
