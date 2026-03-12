use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::storage::repo_manager::get_global_dir;

const SKILL_NAMES: [&str; 6] = [
    "gitnexus-exploring",
    "gitnexus-debugging",
    "gitnexus-impact-analysis",
    "gitnexus-refactoring",
    "gitnexus-guide",
    "gitnexus-cli",
];

#[derive(Debug, Default)]
struct SetupResult {
    configured: Vec<String>,
    skipped: Vec<String>,
    errors: Vec<String>,
}

pub fn run() -> Result<()> {
    println!();
    println!("  GitNexus Setup");
    println!("  ==============");
    println!();

    let global_dir = get_global_dir()?;
    fs::create_dir_all(&global_dir).with_context(|| {
        format!(
            "failed to create global GitNexus directory: {}",
            global_dir.to_string_lossy()
        )
    })?;

    let home = dirs::home_dir().context("failed to determine home directory")?;
    let skills_source = find_skills_source_root();
    let hook_source = find_claude_hook_source();

    let mut result = SetupResult::default();

    setup_cursor(&home, &mut result);
    setup_claude_code(&home, &mut result);
    setup_opencode(&home, &mut result);

    install_claude_code_skills(&home, skills_source.as_deref(), &mut result);
    install_claude_code_hooks(&home, hook_source.as_deref(), &mut result);
    install_cursor_skills(&home, skills_source.as_deref(), &mut result);
    install_opencode_skills(&home, skills_source.as_deref(), &mut result);

    print_summary(&result);
    Ok(())
}

fn setup_cursor(home: &Path, result: &mut SetupResult) {
    let cursor_dir = home.join(".cursor");
    if !dir_exists(&cursor_dir) {
        result.skipped.push("Cursor (not installed)".to_string());
        return;
    }

    let mcp_path = cursor_dir.join("mcp.json");
    match configure_cursor_mcp(&mcp_path) {
        Ok(()) => result.configured.push("Cursor".to_string()),
        Err(err) => result.errors.push(format!("Cursor: {err}")),
    }
}

fn setup_claude_code(home: &Path, result: &mut SetupResult) {
    let claude_dir = home.join(".claude");
    if !dir_exists(&claude_dir) {
        result
            .skipped
            .push("Claude Code (not installed)".to_string());
        return;
    }

    println!("  Claude Code detected. Run this command to add GitNexus MCP:");
    println!();
    println!("    claude mcp add gitnexus -- npx -y gitnexus mcp");
    println!();

    result
        .configured
        .push("Claude Code (MCP manual step printed)".to_string());
}

fn setup_opencode(home: &Path, result: &mut SetupResult) {
    let opencode_dir = home.join(".config").join("opencode");
    if !dir_exists(&opencode_dir) {
        result.skipped.push("OpenCode (not installed)".to_string());
        return;
    }

    let config_path = opencode_dir.join("config.json");
    match configure_opencode_mcp(&config_path) {
        Ok(()) => result.configured.push("OpenCode".to_string()),
        Err(err) => result.errors.push(format!("OpenCode: {err}")),
    }
}

fn install_claude_code_skills(home: &Path, skills_source: Option<&Path>, result: &mut SetupResult) {
    let claude_dir = home.join(".claude");
    if !dir_exists(&claude_dir) {
        return;
    }

    let Some(skills_source) = skills_source else {
        return;
    };

    let skills_dir = claude_dir.join("skills");
    match install_skills_to(&skills_dir, skills_source) {
        Ok(installed) if !installed.is_empty() => result.configured.push(format!(
            "Claude Code skills ({} skills -> ~/.claude/skills/)",
            installed.len()
        )),
        Ok(_) => {}
        Err(err) => result.errors.push(format!("Claude Code skills: {err}")),
    }
}

fn install_cursor_skills(home: &Path, skills_source: Option<&Path>, result: &mut SetupResult) {
    let cursor_dir = home.join(".cursor");
    if !dir_exists(&cursor_dir) {
        return;
    }

    let Some(skills_source) = skills_source else {
        return;
    };

    let skills_dir = cursor_dir.join("skills");
    match install_skills_to(&skills_dir, skills_source) {
        Ok(installed) if !installed.is_empty() => result.configured.push(format!(
            "Cursor skills ({} skills -> ~/.cursor/skills/)",
            installed.len()
        )),
        Ok(_) => {}
        Err(err) => result.errors.push(format!("Cursor skills: {err}")),
    }
}

fn install_opencode_skills(home: &Path, skills_source: Option<&Path>, result: &mut SetupResult) {
    let opencode_dir = home.join(".config").join("opencode");
    if !dir_exists(&opencode_dir) {
        return;
    }

    let Some(skills_source) = skills_source else {
        return;
    };

    let skills_dir = opencode_dir.join("skill");
    match install_skills_to(&skills_dir, skills_source) {
        Ok(installed) if !installed.is_empty() => result.configured.push(format!(
            "OpenCode skills ({} skills -> ~/.config/opencode/skill/)",
            installed.len()
        )),
        Ok(_) => {}
        Err(err) => result.errors.push(format!("OpenCode skills: {err}")),
    }
}

fn install_claude_code_hooks(home: &Path, hook_source: Option<&Path>, result: &mut SetupResult) {
    let claude_dir = home.join(".claude");
    if !dir_exists(&claude_dir) {
        return;
    }

    let Some(hook_source) = hook_source else {
        return;
    };

    match install_claude_hooks(&claude_dir, hook_source) {
        Ok(()) => result
            .configured
            .push("Claude Code hooks (PreToolUse, PostToolUse)".to_string()),
        Err(err) => result.errors.push(format!("Claude Code hooks: {err}")),
    }
}

fn configure_cursor_mcp(config_path: &Path) -> Result<()> {
    let existing = read_json_file(config_path);
    let updated = merge_cursor_config(existing);
    write_json_file(config_path, &updated)
}

fn configure_opencode_mcp(config_path: &Path) -> Result<()> {
    let existing = read_json_file(config_path);
    let updated = merge_opencode_config(existing);
    write_json_file(config_path, &updated)
}

fn merge_cursor_config(existing: Option<Value>) -> Value {
    let mut root = into_object(existing);
    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        *servers = Value::Object(Map::new());
    }
    if let Some(servers_obj) = servers.as_object_mut() {
        servers_obj.insert("gitnexus".to_string(), mcp_entry());
    }
    Value::Object(root)
}

fn merge_opencode_config(existing: Option<Value>) -> Value {
    let mut root = into_object(existing);
    let mcp = root
        .entry("mcp".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !mcp.is_object() {
        *mcp = Value::Object(Map::new());
    }
    if let Some(mcp_obj) = mcp.as_object_mut() {
        mcp_obj.insert("gitnexus".to_string(), mcp_entry());
    }
    Value::Object(root)
}

fn mcp_entry() -> Value {
    if cfg!(target_os = "windows") {
        return json!({
            "command": "cmd",
            "args": ["/c", "npx", "-y", "gitnexus@latest", "mcp"]
        });
    }

    json!({
        "command": "npx",
        "args": ["-y", "gitnexus@latest", "mcp"]
    })
}

fn read_json_file(file_path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(file_path).ok()?;
    serde_json::from_str::<Value>(&raw).ok()
}

fn write_json_file(file_path: &Path, data: &Value) -> Result<()> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory: {}",
                parent.to_string_lossy()
            )
        })?;
    }

    let mut payload =
        serde_json::to_string_pretty(data).context("failed to serialize JSON configuration")?;
    payload.push('\n');

    fs::write(file_path, payload)
        .with_context(|| format!("failed to write {}", file_path.to_string_lossy()))?;
    Ok(())
}

fn into_object(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn dir_exists(path: &Path) -> bool {
    fs::metadata(path)
        .map(|stat| stat.is_dir())
        .unwrap_or(false)
}

fn install_skills_to(target_dir: &Path, skills_root: &Path) -> Result<Vec<String>> {
    let mut installed = Vec::<String>::new();
    for skill_name in SKILL_NAMES {
        let destination_skill_dir = target_dir.join(skill_name);
        let dir_source = skills_root.join(skill_name);
        let directory_skill_file = dir_source.join("SKILL.md");

        if dir_source.is_dir() && directory_skill_file.is_file() {
            copy_dir_recursive(&dir_source, &destination_skill_dir)?;
            installed.push(skill_name.to_string());
            continue;
        }

        let flat_source = skills_root.join(format!("{skill_name}.md"));
        if flat_source.is_file() {
            fs::create_dir_all(&destination_skill_dir).with_context(|| {
                format!(
                    "failed to create skill directory {}",
                    destination_skill_dir.to_string_lossy()
                )
            })?;
            fs::copy(&flat_source, destination_skill_dir.join("SKILL.md")).with_context(|| {
                format!(
                    "failed to copy skill file {}",
                    flat_source.to_string_lossy()
                )
            })?;
            installed.push(skill_name.to_string());
        }
    }
    Ok(installed)
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| {
        format!(
            "failed to create directory {}",
            destination.to_string_lossy()
        )
    })?;

    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to list {}", source.to_string_lossy()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read {}", source.to_string_lossy()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        if entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", source_path.to_string_lossy()))?
            .is_dir()
        {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy {} -> {}",
                    source_path.to_string_lossy(),
                    destination_path.to_string_lossy()
                )
            })?;
        }
    }

    Ok(())
}

fn install_claude_hooks(claude_dir: &Path, hook_source: &Path) -> Result<()> {
    let hook_destination_dir = claude_dir.join("hooks").join("gitnexus");
    fs::create_dir_all(&hook_destination_dir).with_context(|| {
        format!(
            "failed to create hook directory {}",
            hook_destination_dir.to_string_lossy()
        )
    })?;

    let hook_destination = hook_destination_dir.join("gitnexus-hook.cjs");
    fs::copy(hook_source, &hook_destination).with_context(|| {
        format!(
            "failed to copy hook script {} -> {}",
            hook_source.to_string_lossy(),
            hook_destination.to_string_lossy()
        )
    })?;

    let settings_path = claude_dir.join("settings.json");
    let mut settings = read_json_file(&settings_path).unwrap_or_else(|| Value::Object(Map::new()));
    if !settings.is_object() {
        settings = Value::Object(Map::new());
    }

    let hook_path = hook_destination.to_string_lossy().replace('\\', "/");
    let hook_cmd = format!("node \"{}\"", hook_path.replace('"', "\\\""));

    ensure_claude_hook_entry(
        &mut settings,
        "PreToolUse",
        "Grep|Glob|Bash",
        &hook_cmd,
        10,
        "Enriching with GitNexus graph context...",
    );
    ensure_claude_hook_entry(
        &mut settings,
        "PostToolUse",
        "Bash",
        &hook_cmd,
        10,
        "Checking GitNexus index freshness...",
    );

    write_json_file(&settings_path, &settings)
}

fn ensure_claude_hook_entry(
    settings: &mut Value,
    event_name: &str,
    matcher: &str,
    hook_cmd: &str,
    timeout: u64,
    status_message: &str,
) {
    let Some(settings_obj) = settings.as_object_mut() else {
        return;
    };

    let hooks = settings_obj
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        *hooks = Value::Object(Map::new());
    }

    let Some(hooks_obj) = hooks.as_object_mut() else {
        return;
    };
    let event_entries = hooks_obj
        .entry(event_name.to_string())
        .or_insert_with(|| Value::Array(vec![]));
    if !event_entries.is_array() {
        *event_entries = Value::Array(vec![]);
    }

    let Some(event_arr) = event_entries.as_array_mut() else {
        return;
    };
    let has_gitnexus = event_arr.iter().any(event_entry_has_gitnexus_hook);
    if has_gitnexus {
        return;
    }

    event_arr.push(json!({
        "matcher": matcher,
        "hooks": [
            {
                "type": "command",
                "command": hook_cmd,
                "timeout": timeout,
                "statusMessage": status_message
            }
        ]
    }));
}

fn event_entry_has_gitnexus_hook(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.contains("gitnexus-hook"))
            })
        })
        .unwrap_or(false)
}

fn find_skills_source_root() -> Option<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in candidate_roots() {
        candidates.push(root.join("skills"));
        candidates.push(root.join("GitNexus").join("gitnexus").join("skills"));
    }

    dedupe_paths(candidates)
        .into_iter()
        .find(|candidate| candidate.is_dir() && skills_root_has_gitnexus_skills(candidate))
}

fn find_claude_hook_source() -> Option<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for root in candidate_roots() {
        candidates.push(root.join("hooks").join("claude").join("gitnexus-hook.cjs"));
        candidates.push(
            root.join("GitNexus")
                .join("gitnexus")
                .join("hooks")
                .join("claude")
                .join("gitnexus-hook.cjs"),
        );
    }

    dedupe_paths(candidates)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::<PathBuf>::new();

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        roots.push(bin_dir.to_path_buf());
        if let Some(parent) = bin_dir.parent() {
            roots.push(parent.to_path_buf());
            if let Some(grand_parent) = parent.parent() {
                roots.push(grand_parent.to_path_buf());
            }
        }
    }

    dedupe_paths(roots)
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::<PathBuf>::new();
    let mut deduped = Vec::<PathBuf>::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

fn skills_root_has_gitnexus_skills(root: &Path) -> bool {
    SKILL_NAMES.iter().any(|skill_name| {
        root.join(skill_name).join("SKILL.md").is_file()
            || root.join(format!("{skill_name}.md")).is_file()
    })
}

fn print_summary(result: &SetupResult) {
    if !result.configured.is_empty() {
        println!("  Configured:");
        for item in &result.configured {
            println!("    + {item}");
        }
    }

    if !result.skipped.is_empty() {
        println!();
        println!("  Skipped:");
        for item in &result.skipped {
            println!("    - {item}");
        }
    }

    if !result.errors.is_empty() {
        println!();
        println!("  Errors:");
        for item in &result.errors {
            println!("    ! {item}");
        }
    }

    println!();
    println!("  Summary:");
    let mcp_entries = result
        .configured
        .iter()
        .filter(|item| !item.contains("skills") && !item.contains("hooks"))
        .cloned()
        .collect::<Vec<_>>();
    let skill_entries = result
        .configured
        .iter()
        .filter(|item| item.contains("skills"))
        .cloned()
        .collect::<Vec<_>>();
    println!(
        "    MCP configured for: {}",
        if mcp_entries.is_empty() {
            "none".to_string()
        } else {
            mcp_entries.join(", ")
        }
    );
    println!(
        "    Skills installed to: {}",
        if skill_entries.is_empty() {
            "none".to_string()
        } else {
            skill_entries.join(", ")
        }
    );
    println!();
    println!("  Next steps:");
    println!("    1. cd into any git repo");
    println!("    2. Run: gitnexus analyze");
    println!("    3. Open the repo in your editor -- MCP is ready!");
    println!();
}
