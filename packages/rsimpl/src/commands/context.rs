use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct ContextOptions {
    pub repo: Option<String>,
    pub uid: Option<String>,
    pub file: Option<String>,
    pub content: bool,
}

pub fn run(name: Option<&str>, options: ContextOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(name) = name {
        args.push(name.to_string());
    }

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    if let Some(uid) = options.uid {
        args.push("--uid".to_string());
        args.push(uid);
    }

    if let Some(file) = options.file {
        args.push("--file".to_string());
        args.push(file);
    }

    if options.content {
        args.push("--content".to_string());
    }

    run_ts_cli("context", &args)
}
