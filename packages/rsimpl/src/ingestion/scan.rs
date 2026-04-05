use std::path::Path;

use anyhow::{Context, Result};
use walkdir::{DirEntry, WalkDir};

use super::{MAX_FILE_SIZE_BYTES, ScanResult, ScannedFile, SupportedLanguage};

pub(super) fn scan_repository_paths(repo_path: &Path) -> Result<ScanResult> {
    let mut files = Vec::<ScannedFile>::new();
    let mut skipped_large_files = 0u64;

    for entry in WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|entry| should_descend(entry))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path();
        let rel_path = abs_path
            .strip_prefix(repo_path)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace('\\', "/");

        if should_ignore_path(&rel_path) {
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for {rel_path}"))?;
        if metadata.len() > MAX_FILE_SIZE_BYTES {
            skipped_large_files += 1;
            continue;
        }

        files.push(ScannedFile {
            language: detect_language_from_filename(&rel_path),
            path: rel_path,
            size_bytes: metadata.len(),
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ScanResult {
        files,
        skipped_large_files,
    })
}

pub(super) fn should_descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }

    let dir_name = entry.file_name().to_string_lossy();
    !is_ignored_dir(&dir_name)
}

pub(super) fn should_ignore_path(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    let mut parts = normalized.split('/').peekable();

    while let Some(part) = parts.next() {
        if parts.peek().is_some() && is_ignored_dir(part) {
            return true;
        }
    }

    let file_name = normalized.rsplit('/').next().unwrap_or_default();
    if file_name.is_empty() {
        return false;
    }

    let lower = file_name.to_ascii_lowercase();
    if is_ignored_file(&lower) {
        return true;
    }

    if has_ignored_extension(&lower) {
        return true;
    }

    if lower.contains(".bundle.")
        || lower.contains(".chunk.")
        || lower.contains(".generated.")
        || lower.ends_with(".d.ts")
    {
        return true;
    }

    false
}

pub(super) fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".gitnexus"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "dist"
            | "build"
            | "out"
            | "output"
            | "target"
            | ".next"
            | ".nuxt"
            | ".output"
            | ".turbo"
            | ".svelte-kit"
            | ".cache"
            | "coverage"
            | ".nyc_output"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".venv"
            | "venv"
            | "vendor"
            | ".idea"
            | ".vscode"
            | ".github"
            | ".gitlab"
            | ".circleci"
            | ".terraform"
            | ".serverless"
    )
}

pub(super) fn is_ignored_file(file_name_lower: &str) -> bool {
    matches!(
        file_name_lower,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "composer.lock"
            | "gemfile.lock"
            | "poetry.lock"
            | "cargo.lock"
            | "go.sum"
            | ".gitignore"
            | ".gitattributes"
            | ".npmrc"
            | ".yarnrc"
            | ".editorconfig"
            | ".dockerignore"
            | ".prettierignore"
            | ".eslintignore"
            | "license"
            | "license.md"
            | "license.txt"
            | "changelog"
            | "changelog.md"
            | "code_of_conduct.md"
            | "contributing.md"
            | "security.md"
            | "thumbs.db"
            | ".ds_store"
    )
}

pub(super) fn has_ignored_extension(file_name_lower: &str) -> bool {
    const IGNORED_EXTENSIONS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp", ".bmp", ".tiff", ".pdf", ".mp4",
        ".mp3", ".wav", ".zip", ".tar", ".gz", ".rar", ".7z", ".exe", ".dll", ".so", ".dylib",
        ".class", ".jar", ".war", ".pyc", ".wasm", ".node", ".db", ".sqlite", ".sqlite3", ".map",
        ".lock", ".pem", ".key", ".crt", ".csv", ".tsv", ".parquet", ".bin",
    ];

    IGNORED_EXTENSIONS
        .iter()
        .any(|ext| file_name_lower.ends_with(ext))
}

pub(super) fn detect_language_from_filename(filename: &str) -> Option<SupportedLanguage> {
    let lower = filename.to_ascii_lowercase();

    if lower.ends_with(".tsx") || lower.ends_with(".ts") {
        return Some(SupportedLanguage::TypeScript);
    }
    if lower.ends_with(".jsx") || lower.ends_with(".js") {
        return Some(SupportedLanguage::JavaScript);
    }
    if lower.ends_with(".py") {
        return Some(SupportedLanguage::Python);
    }
    if lower.ends_with(".java") {
        return Some(SupportedLanguage::Java);
    }
    if lower.ends_with(".c") || lower.ends_with(".h") {
        return Some(SupportedLanguage::C);
    }
    if lower.ends_with(".cpp")
        || lower.ends_with(".cc")
        || lower.ends_with(".cxx")
        || lower.ends_with(".hpp")
        || lower.ends_with(".hxx")
        || lower.ends_with(".hh")
    {
        return Some(SupportedLanguage::Cpp);
    }
    if lower.ends_with(".cs") {
        return Some(SupportedLanguage::CSharp);
    }
    if lower.ends_with(".go") {
        return Some(SupportedLanguage::Go);
    }
    if lower.ends_with(".rs") {
        return Some(SupportedLanguage::Rust);
    }
    if lower.ends_with(".kt") || lower.ends_with(".kts") {
        return Some(SupportedLanguage::Kotlin);
    }
    if lower.ends_with(".php")
        || lower.ends_with(".phtml")
        || lower.ends_with(".php3")
        || lower.ends_with(".php4")
        || lower.ends_with(".php5")
        || lower.ends_with(".php8")
    {
        return Some(SupportedLanguage::PHP);
    }
    if lower.ends_with(".swift") {
        return Some(SupportedLanguage::Swift);
    }

    None
}
