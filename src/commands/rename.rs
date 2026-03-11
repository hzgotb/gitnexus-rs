use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone)]
pub struct RenameOptions {
    pub symbol_name: Option<String>,
    pub symbol_uid: Option<String>,
    pub new_name: String,
    pub file_path: Option<String>,
    pub repo: Option<String>,
    pub dry_run: bool,
}

pub fn run(options: RenameOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(symbol_name) = options.symbol_name {
        args.push("--symbol-name".to_string());
        args.push(symbol_name);
    }

    if let Some(symbol_uid) = options.symbol_uid {
        args.push("--symbol-uid".to_string());
        args.push(symbol_uid);
    }

    args.push("--new-name".to_string());
    args.push(options.new_name);

    if let Some(file_path) = options.file_path {
        args.push("--file-path".to_string());
        args.push(file_path);
    }

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    args.push("--dry-run".to_string());
    args.push(if options.dry_run {
        "true".to_string()
    } else {
        "false".to_string()
    });

    run_ts_cli("rename", &args)
}
