use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::commands::local_tools::try_run_native_tool;

pub fn run_ts_cli(subcommand: &str, args: &[String]) -> Result<()> {
    if try_run_native_tool(subcommand, args)? {
        return Ok(());
    }

    let mut direct_args = vec![subcommand.to_string()];
    direct_args.extend(args.iter().cloned());

    if run_command_if_exists("gitnexus", &direct_args, None)? {
        return Ok(());
    }

    let cwd = std::env::current_dir().context("failed to read current working directory")?;

    if let Some(local_pkg) = find_local_gitnexus_package(&cwd) {
        if let Some((command, mut command_args, command_cwd)) = build_local_command(&local_pkg) {
            command_args.push(subcommand.to_string());
            command_args.extend(args.iter().cloned());
            if run_command_if_exists(&command, &command_args, Some(&command_cwd))? {
                return Ok(());
            }
        }
    }

    let mut npx_args = vec![
        "-y".to_string(),
        "gitnexus@latest".to_string(),
        subcommand.to_string(),
    ];
    npx_args.extend(args.iter().cloned());
    run_command("npx", &npx_args, None)
}

fn build_local_command(pkg_path: &Path) -> Option<(String, Vec<String>, PathBuf)> {
    let dist_cli = pkg_path.join("dist").join("cli").join("index.js");
    if dist_cli.is_file() {
        return Some((
            "node".to_string(),
            vec![dist_cli.to_string_lossy().to_string()],
            pkg_path.to_path_buf(),
        ));
    }

    let src_cli = pkg_path.join("src").join("cli").join("index.ts");
    if src_cli.is_file() {
        let tsx_bin = if cfg!(windows) {
            pkg_path.join("node_modules").join(".bin").join("tsx.cmd")
        } else {
            pkg_path.join("node_modules").join(".bin").join("tsx")
        };

        if tsx_bin.is_file() {
            return Some((
                tsx_bin.to_string_lossy().to_string(),
                vec![src_cli.to_string_lossy().to_string()],
                pkg_path.to_path_buf(),
            ));
        }

        return Some((
            "npx".to_string(),
            vec![
                "-y".to_string(),
                "tsx".to_string(),
                src_cli.to_string_lossy().to_string(),
            ],
            pkg_path.to_path_buf(),
        ));
    }

    None
}

fn run_command(command: &str, args: &[String], cwd: Option<&Path>) -> Result<()> {
    if run_command_if_exists(command, args, cwd)? {
        return Ok(());
    }
    bail!("delegated command not found: {command}")
}

fn run_command_if_exists(command: &str, args: &[String], cwd: Option<&Path>) -> Result<bool> {
    let mut child = Command::new(command);
    child.args(args);
    if let Some(path) = cwd {
        child.current_dir(path);
    }
    let status = match child
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to start delegated command: {} {}",
                    command,
                    args.join(" ")
                )
            });
        }
    };

    if status.success() {
        return Ok(true);
    }

    if let Some(code) = status.code() {
        bail!("delegated command exited with status code {code}");
    }

    bail!("delegated command terminated by signal")
}

fn find_local_gitnexus_package(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();

    loop {
        if is_gitnexus_package(&current) {
            return Some(current.clone());
        }

        let nested = current.join("gitnexus");
        if is_gitnexus_package(&nested) {
            return Some(nested);
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }

    None
}

fn is_gitnexus_package(path: &Path) -> bool {
    path.join("package.json").is_file()
        && (path.join("dist").join("cli").join("index.js").is_file()
            || path.join("src").join("cli").join("index.ts").is_file())
}
