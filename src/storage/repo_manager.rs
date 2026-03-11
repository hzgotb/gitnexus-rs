use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const GITNEXUS_DIR: &str = ".gitnexus";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStats {
    pub files: Option<u64>,
    pub nodes: Option<u64>,
    pub edges: Option<u64>,
    pub communities: Option<u64>,
    pub processes: Option<u64>,
    pub embeddings: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMeta {
    pub repo_path: String,
    pub last_commit: String,
    pub indexed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RepoStats>,
}

#[derive(Debug, Clone)]
pub struct StoragePaths {
    pub storage_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct IndexedRepo {
    pub repo_path: PathBuf,
    pub storage_path: PathBuf,
    pub meta: RepoMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub name: String,
    pub path: String,
    pub storage_path: String,
    pub indexed_at: String,
    pub last_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RepoStats>,
}

pub fn get_storage_path(repo_path: &Path) -> PathBuf {
    resolve_path(repo_path).join(GITNEXUS_DIR)
}

pub fn get_storage_paths(repo_path: &Path) -> StoragePaths {
    let storage_path = get_storage_path(repo_path);
    StoragePaths { storage_path }
}

pub fn load_meta(storage_path: &Path) -> Result<Option<RepoMeta>> {
    let meta_path = storage_path.join("meta.json");
    if !meta_path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("failed to read {}", meta_path.to_string_lossy()))?;
    let meta = serde_json::from_str::<RepoMeta>(&raw).context("failed to parse meta.json")?;
    Ok(Some(meta))
}

pub fn save_meta(storage_path: &Path, meta: &RepoMeta) -> Result<()> {
    std::fs::create_dir_all(storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage_path.to_string_lossy()
        )
    })?;

    let meta_path = storage_path.join("meta.json");
    let data = serde_json::to_string_pretty(meta)?;
    std::fs::write(&meta_path, data)
        .with_context(|| format!("failed to write {}", meta_path.to_string_lossy()))?;
    Ok(())
}

pub fn load_repo(repo_path: &Path) -> Result<Option<IndexedRepo>> {
    let resolved = resolve_path(repo_path);
    let paths = get_storage_paths(&resolved);
    let meta = match load_meta(&paths.storage_path)? {
        Some(meta) => meta,
        None => return Ok(None),
    };

    Ok(Some(IndexedRepo {
        repo_path: resolved,
        storage_path: paths.storage_path,
        meta,
    }))
}

pub fn find_repo(start_path: &Path) -> Result<Option<IndexedRepo>> {
    let mut current = resolve_path(start_path);

    loop {
        if let Some(repo) = load_repo(&current)? {
            return Ok(Some(repo));
        }

        let Some(parent) = current.parent() else {
            break;
        };

        if parent == current {
            break;
        }

        current = parent.to_path_buf();
    }

    Ok(None)
}

pub fn add_to_gitignore(repo_path: &Path) -> Result<()> {
    let gitignore_path = repo_path.join(".gitignore");

    if !gitignore_path.exists() {
        std::fs::write(&gitignore_path, format!("{GITNEXUS_DIR}\n"))?;
        return Ok(());
    }

    let content = std::fs::read_to_string(&gitignore_path)
        .with_context(|| format!("failed to read {}", gitignore_path.to_string_lossy()))?;

    if content.contains(GITNEXUS_DIR) {
        return Ok(());
    }

    let mut new_content = content;
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(GITNEXUS_DIR);
    new_content.push('\n');

    std::fs::write(&gitignore_path, new_content)
        .with_context(|| format!("failed to write {}", gitignore_path.to_string_lossy()))?;
    Ok(())
}

pub fn get_global_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("failed to determine home directory")?;
    Ok(home.join(".gitnexus"))
}

pub fn get_global_registry_path() -> Result<PathBuf> {
    Ok(get_global_dir()?.join("registry.json"))
}

pub fn read_registry() -> Result<Vec<RegistryEntry>> {
    let path = get_global_registry_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.to_string_lossy()))?;

    match serde_json::from_str::<Vec<RegistryEntry>>(&raw) {
        Ok(entries) => Ok(entries),
        Err(_) => Ok(Vec::new()),
    }
}

pub fn register_repo(repo_path: &Path, meta: &RepoMeta) -> Result<()> {
    let resolved = resolve_path(repo_path);
    let name = resolved
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let storage_path = get_storage_path(&resolved);

    let mut entries = read_registry()?;
    let idx = entries
        .iter()
        .position(|entry| path_strings_equal(&entry.path, &resolved));

    let entry = RegistryEntry {
        name,
        path: resolved.to_string_lossy().to_string(),
        storage_path: storage_path.to_string_lossy().to_string(),
        indexed_at: meta.indexed_at.clone(),
        last_commit: meta.last_commit.clone(),
        stats: meta.stats.clone(),
    };

    if let Some(i) = idx {
        entries[i] = entry;
    } else {
        entries.push(entry);
    }

    write_registry(&entries)
}

pub fn unregister_repo(repo_path: &Path) -> Result<()> {
    let resolved = resolve_path(repo_path);
    let mut entries = read_registry()?;
    entries.retain(|entry| !path_strings_equal(&entry.path, &resolved));
    write_registry(&entries)
}

pub fn list_registered_repos(validate: bool) -> Result<Vec<RegistryEntry>> {
    let entries = read_registry()?;
    if !validate {
        return Ok(entries);
    }

    let valid: Vec<RegistryEntry> = entries
        .iter()
        .filter(|entry| Path::new(&entry.storage_path).join("meta.json").exists())
        .cloned()
        .collect();

    if valid.len() != entries.len() {
        write_registry(&valid)?;
    }

    Ok(valid)
}

fn write_registry(entries: &[RegistryEntry]) -> Result<()> {
    let dir = get_global_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "failed to create global gitnexus directory {}",
            dir.to_string_lossy()
        )
    })?;

    let path = get_global_registry_path()?;
    let content = serde_json::to_string_pretty(entries)?;
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.to_string_lossy()))?;
    Ok(())
}

fn resolve_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn normalize_for_compare(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| resolve_path(path))
}

fn path_strings_equal(path_a: &str, path_b: &Path) -> bool {
    let a = normalize_for_compare(Path::new(path_a));
    let b = normalize_for_compare(path_b);

    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(b.to_string_lossy().as_ref())
    } else {
        a == b
    }
}
