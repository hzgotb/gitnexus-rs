use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

pub fn run() -> Result<()> {
    run_ts_cli("setup", &[])
}
