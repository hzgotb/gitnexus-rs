use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

const MAX_FILE_SIZE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportedLanguage {
    JavaScript,
    TypeScript,
    Python,
    Java,
    C,
    Cpp,
    CSharp,
    Go,
    Rust,
    PHP,
    Kotlin,
    Swift,
}

impl SupportedLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::PHP => "php",
            Self::Kotlin => "kotlin",
            Self::Swift => "swift",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    pub path: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<SupportedLanguage>,
}

#[derive(Debug, Clone)]
struct ParsedSymbol {
    id: String,
    label: String,
    name: String,
    file_path: String,
    start_line: u64,
    language: SupportedLanguage,
}

#[derive(Debug, Clone)]
struct ParsedImport {
    source_file: String,
    target_file: String,
}

#[derive(Debug, Clone)]
struct ParsedCall {
    source_symbol_id: String,
    target_symbol_id: String,
    confidence: f32,
}

#[derive(Debug, Clone)]
struct ParsedHeritage {
    source_symbol_id: String,
    target_symbol_id: String,
    rel_type: String,
    confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureNode {
    pub id: String,
    pub label: String,
    pub name: String,
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureRelationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    #[serde(rename = "type")]
    pub rel_type: String,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureGraph {
    pub nodes: Vec<StructureNode>,
    pub relationships: Vec<StructureRelationship>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionStats {
    pub total_files: u64,
    pub parseable_files: u64,
    pub symbols: u64,
    pub skipped_large_files: u64,
    pub nodes: u64,
    pub edges: u64,
    #[serde(default)]
    pub communities: u64,
    #[serde(default)]
    pub processes: u64,
    pub language_breakdown: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicCommunity {
    pub id: String,
    pub file_count: u64,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicProcess {
    pub id: String,
    pub entry_symbol_id: String,
    pub symbol_count: u64,
    pub step_count: u64,
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionResult {
    pub scanned_files: Vec<ScannedFile>,
    pub graph: StructureGraph,
    pub stats: IngestionStats,
    pub communities: Vec<HeuristicCommunity>,
    pub processes: Vec<HeuristicProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestionReport {
    pub phase: String,
    pub generated_at: String,
    pub stats: IngestionStats,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub communities: Vec<HeuristicCommunity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processes: Vec<HeuristicProcess>,
}

#[derive(Debug)]
struct ScanResult {
    files: Vec<ScannedFile>,
    skipped_large_files: u64,
}

pub fn run_ingestion_pipeline(repo_path: &Path) -> Result<IngestionResult> {
    let scan = scan_repository_paths(repo_path)?;
    let symbols = parse_symbols(repo_path, &scan.files)?;
    let imports = parse_imports(repo_path, &scan.files)?;
    let calls = parse_calls(repo_path, &scan.files, &symbols, &imports)?;
    let heritages = parse_heritage(repo_path, &scan.files, &symbols, &imports)?;
    let graph = build_structure_graph(&scan.files, &symbols, &imports, &calls, &heritages);
    let communities = build_community_summaries(&graph);
    let processes = build_process_summaries(&graph);

    let parseable_files = scan
        .files
        .iter()
        .filter(|file| file.language.is_some())
        .count() as u64;

    let mut language_breakdown = BTreeMap::<String, u64>::new();
    for file in &scan.files {
        if let Some(language) = file.language {
            *language_breakdown
                .entry(language.as_str().to_string())
                .or_insert(0) += 1;
        }
    }

    let stats = IngestionStats {
        total_files: scan.files.len() as u64,
        parseable_files,
        symbols: symbols.len() as u64,
        skipped_large_files: scan.skipped_large_files,
        nodes: graph.nodes.len() as u64,
        edges: graph.relationships.len() as u64,
        communities: communities.len() as u64,
        processes: processes.len() as u64,
        language_breakdown,
    };

    Ok(IngestionResult {
        scanned_files: scan.files,
        graph,
        stats,
        communities,
        processes,
    })
}

pub fn save_ingestion_report(storage_path: &Path, result: &IngestionResult) -> Result<()> {
    let report = IngestionReport {
        phase: "structure+heuristic-relations".to_string(),
        generated_at: Utc::now().to_rfc3339(),
        stats: result.stats.clone(),
        communities: result.communities.clone(),
        processes: result.processes.clone(),
    };

    std::fs::create_dir_all(storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage_path.to_string_lossy()
        )
    })?;

    let report_path = storage_path.join("ingestion.json");
    let content = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, content)
        .with_context(|| format!("failed to write {}", report_path.to_string_lossy()))?;
    Ok(())
}

fn scan_repository_paths(repo_path: &Path) -> Result<ScanResult> {
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

fn build_structure_graph(
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
    calls: &[ParsedCall],
    heritages: &[ParsedHeritage],
) -> StructureGraph {
    let mut nodes = Vec::<StructureNode>::new();
    let mut relationships = Vec::<StructureRelationship>::new();
    let mut node_ids = HashSet::<String>::new();
    let mut relationship_ids = HashSet::<String>::new();

    for file in files {
        let parts: Vec<&str> = file
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            continue;
        }

        let mut current_path = String::new();
        let mut parent_id: Option<String> = None;

        for (idx, part) in parts.iter().enumerate() {
            if !current_path.is_empty() {
                current_path.push('/');
            }
            current_path.push_str(part);

            let label = if idx == parts.len() - 1 {
                "File"
            } else {
                "Folder"
            };

            let node_id = generate_id(label, &current_path);
            if node_ids.insert(node_id.clone()) {
                nodes.push(StructureNode {
                    id: node_id.clone(),
                    label: label.to_string(),
                    name: (*part).to_string(),
                    file_path: current_path.clone(),
                    start_line: None,
                    language: None,
                });
            }

            if let Some(parent) = &parent_id {
                let rel_id = generate_id("CONTAINS", &format!("{parent}->{node_id}"));
                if relationship_ids.insert(rel_id.clone()) {
                    relationships.push(StructureRelationship {
                        id: rel_id,
                        source_id: parent.clone(),
                        target_id: node_id.clone(),
                        rel_type: "CONTAINS".to_string(),
                        confidence: 1.0,
                        reason: String::new(),
                    });
                }
            }

            parent_id = Some(node_id);
        }
    }

    for symbol in symbols {
        if node_ids.insert(symbol.id.clone()) {
            nodes.push(StructureNode {
                id: symbol.id.clone(),
                label: symbol.label.clone(),
                name: symbol.name.clone(),
                file_path: symbol.file_path.clone(),
                start_line: Some(symbol.start_line),
                language: Some(symbol.language.as_str().to_string()),
            });
        }

        let file_id = generate_id("File", &symbol.file_path);
        let rel_id = generate_id("DEFINES", &format!("{file_id}->{}", symbol.id));
        if relationship_ids.insert(rel_id.clone()) {
            relationships.push(StructureRelationship {
                id: rel_id,
                source_id: file_id,
                target_id: symbol.id.clone(),
                rel_type: "DEFINES".to_string(),
                confidence: 1.0,
                reason: String::new(),
            });
        }
    }

    for import in imports {
        let source_id = generate_id("File", &import.source_file);
        let target_id = generate_id("File", &import.target_file);
        if !node_ids.contains(&source_id) || !node_ids.contains(&target_id) {
            continue;
        }

        let rel_id = generate_id("IMPORTS", &format!("{source_id}->{target_id}"));
        if relationship_ids.insert(rel_id.clone()) {
            relationships.push(StructureRelationship {
                id: rel_id,
                source_id,
                target_id,
                rel_type: "IMPORTS".to_string(),
                confidence: 0.95,
                reason: String::new(),
            });
        }
    }

    for call in calls {
        if !node_ids.contains(&call.source_symbol_id) || !node_ids.contains(&call.target_symbol_id)
        {
            continue;
        }

        let rel_id = generate_id(
            "CALLS",
            &format!("{}->{}", call.source_symbol_id, call.target_symbol_id),
        );
        if relationship_ids.insert(rel_id.clone()) {
            relationships.push(StructureRelationship {
                id: rel_id,
                source_id: call.source_symbol_id.clone(),
                target_id: call.target_symbol_id.clone(),
                rel_type: "CALLS".to_string(),
                confidence: call.confidence,
                reason: String::new(),
            });
        }
    }

    for heritage in heritages {
        if !node_ids.contains(&heritage.source_symbol_id)
            || !node_ids.contains(&heritage.target_symbol_id)
        {
            continue;
        }

        let rel_id = generate_id(
            &heritage.rel_type,
            &format!(
                "{}->{}",
                heritage.source_symbol_id, heritage.target_symbol_id
            ),
        );
        if relationship_ids.insert(rel_id.clone()) {
            relationships.push(StructureRelationship {
                id: rel_id,
                source_id: heritage.source_symbol_id.clone(),
                target_id: heritage.target_symbol_id.clone(),
                rel_type: heritage.rel_type.clone(),
                confidence: heritage.confidence,
                reason: String::new(),
            });
        }
    }

    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    relationships.sort_by(|a, b| a.id.cmp(&b.id));

    StructureGraph {
        nodes,
        relationships,
    }
}

fn generate_id(label: &str, name: &str) -> String {
    format!("{label}:{name}")
}

fn should_descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }

    let dir_name = entry.file_name().to_string_lossy();
    !is_ignored_dir(&dir_name)
}

fn should_ignore_path(file_path: &str) -> bool {
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

fn is_ignored_dir(name: &str) -> bool {
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

fn is_ignored_file(file_name_lower: &str) -> bool {
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

fn has_ignored_extension(file_name_lower: &str) -> bool {
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

pub fn detect_language_from_filename(filename: &str) -> Option<SupportedLanguage> {
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

fn parse_symbols(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedSymbol>> {
    let mut symbols = Vec::<ParsedSymbol>::new();

    for file in files {
        let Some(language) = file.language else {
            continue;
        };

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        let extracted = extract_symbols_from_content(&content, &file.path, language);
        symbols.extend(extracted);
    }

    symbols.sort_by(|a, b| a.id.cmp(&b.id));
    symbols.dedup_by(|a, b| a.id == b.id);
    Ok(symbols)
}

fn parse_imports(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedImport>> {
    let file_set: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let mut imports = Vec::<ParsedImport>::new();
    let mut seen = HashSet::<String>::new();

    for file in files {
        let Some(language) = file.language else {
            continue;
        };

        if !matches!(
            language,
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript
        ) {
            continue;
        }

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        for specifier in extract_import_specifiers(&content, language) {
            if !specifier.starts_with('.') {
                continue;
            }

            let Some(target_file) =
                resolve_relative_import_path(&file.path, &specifier, language, &file_set)
            else {
                continue;
            };

            let key = format!("{}->{target_file}", file.path);
            if !seen.insert(key) {
                continue;
            }

            imports.push(ParsedImport {
                source_file: file.path.clone(),
                target_file,
            });
        }
    }

    imports.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then(a.target_file.cmp(&b.target_file))
    });
    Ok(imports)
}

fn parse_calls(
    repo_path: &Path,
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
) -> Result<Vec<ParsedCall>> {
    let mut symbols_by_file = BTreeMap::<String, Vec<&ParsedSymbol>>::new();
    for symbol in symbols {
        symbols_by_file
            .entry(symbol.file_path.clone())
            .or_default()
            .push(symbol);
    }
    for file_symbols in symbols_by_file.values_mut() {
        file_symbols.sort_by_key(|s| s.start_line);
    }

    let mut symbols_by_file_name = BTreeMap::<String, BTreeMap<String, Vec<&ParsedSymbol>>>::new();
    for symbol in symbols {
        symbols_by_file_name
            .entry(symbol.file_path.clone())
            .or_default()
            .entry(symbol.name.clone())
            .or_default()
            .push(symbol);
    }

    let mut imports_by_source = BTreeMap::<String, Vec<String>>::new();
    for import in imports {
        imports_by_source
            .entry(import.source_file.clone())
            .or_default()
            .push(import.target_file.clone());
    }

    let mut seen = HashSet::<String>::new();
    let mut calls = Vec::<ParsedCall>::new();

    for file in files {
        let Some(language) = file.language else {
            continue;
        };

        if !matches!(
            language,
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript
        ) {
            continue;
        }

        let Some(file_symbols) = symbols_by_file.get(&file.path) else {
            continue;
        };

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        for (idx, raw_line) in content.lines().enumerate() {
            let line_no = (idx + 1) as u64;
            let line = strip_inline_comment(raw_line).trim();
            if line.is_empty() || looks_like_definition_line(line) {
                continue;
            }

            let Some(caller_symbol) = find_caller_symbol(file_symbols, line_no) else {
                continue;
            };

            for callee_name in extract_call_identifiers(line) {
                if is_non_call_keyword(&callee_name) || callee_name == caller_symbol.name {
                    continue;
                }

                let Some((target_symbol_id, confidence)) = resolve_callee_symbol_id(
                    &file.path,
                    &callee_name,
                    &symbols_by_file_name,
                    &imports_by_source,
                ) else {
                    continue;
                };

                if target_symbol_id == caller_symbol.id {
                    continue;
                }

                let key = format!("{}->{target_symbol_id}", caller_symbol.id);
                if !seen.insert(key) {
                    continue;
                }

                calls.push(ParsedCall {
                    source_symbol_id: caller_symbol.id.clone(),
                    target_symbol_id,
                    confidence,
                });
            }
        }
    }

    calls.sort_by(|a, b| {
        a.source_symbol_id
            .cmp(&b.source_symbol_id)
            .then(a.target_symbol_id.cmp(&b.target_symbol_id))
    });
    Ok(calls)
}

fn parse_heritage(
    repo_path: &Path,
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
) -> Result<Vec<ParsedHeritage>> {
    let mut symbols_by_file_name = BTreeMap::<String, BTreeMap<String, Vec<&ParsedSymbol>>>::new();
    for symbol in symbols {
        symbols_by_file_name
            .entry(symbol.file_path.clone())
            .or_default()
            .entry(symbol.name.clone())
            .or_default()
            .push(symbol);
    }

    let mut imports_by_source = BTreeMap::<String, Vec<String>>::new();
    for import in imports {
        imports_by_source
            .entry(import.source_file.clone())
            .or_default()
            .push(import.target_file.clone());
    }

    let mut seen = HashSet::<String>::new();
    let mut heritages = Vec::<ParsedHeritage>::new();

    for file in files {
        let Some(language) = file.language else {
            continue;
        };

        if !matches!(
            language,
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript
        ) {
            continue;
        }

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        for raw_line in content.lines() {
            let line = strip_inline_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            if let Some(source_name) = extract_identifier_after_keyword(line, "class ") {
                let Some(source_symbol_id) = resolve_symbol_id_in_file(
                    &file.path,
                    &source_name,
                    Some("Class"),
                    &symbols_by_file_name,
                ) else {
                    continue;
                };

                let extends_clause = extract_clause(line, "extends", &["implements", "{"]);
                if let Some(clause) = extends_clause {
                    for target_name in parse_type_identifier_list(&clause) {
                        let Some((target_symbol_id, confidence)) = resolve_related_symbol_id(
                            &file.path,
                            &target_name,
                            Some("Class"),
                            &symbols_by_file_name,
                            &imports_by_source,
                        ) else {
                            continue;
                        };

                        if target_symbol_id == source_symbol_id {
                            continue;
                        }

                        let key = format!("EXTENDS:{}->{target_symbol_id}", source_symbol_id);
                        if !seen.insert(key) {
                            continue;
                        }

                        heritages.push(ParsedHeritage {
                            source_symbol_id: source_symbol_id.clone(),
                            target_symbol_id,
                            rel_type: "EXTENDS".to_string(),
                            confidence,
                        });
                    }
                }

                let implements_clause = extract_clause(line, "implements", &["{"]);
                if let Some(clause) = implements_clause {
                    for target_name in parse_type_identifier_list(&clause) {
                        let Some((target_symbol_id, confidence)) = resolve_related_symbol_id(
                            &file.path,
                            &target_name,
                            Some("Interface"),
                            &symbols_by_file_name,
                            &imports_by_source,
                        ) else {
                            continue;
                        };

                        if target_symbol_id == source_symbol_id {
                            continue;
                        }

                        let key = format!("IMPLEMENTS:{}->{target_symbol_id}", source_symbol_id);
                        if !seen.insert(key) {
                            continue;
                        }

                        heritages.push(ParsedHeritage {
                            source_symbol_id: source_symbol_id.clone(),
                            target_symbol_id,
                            rel_type: "IMPLEMENTS".to_string(),
                            confidence,
                        });
                    }
                }

                continue;
            }

            if let Some(source_name) = extract_identifier_after_keyword(line, "interface ") {
                let Some(source_symbol_id) = resolve_symbol_id_in_file(
                    &file.path,
                    &source_name,
                    Some("Interface"),
                    &symbols_by_file_name,
                ) else {
                    continue;
                };

                let extends_clause = extract_clause(line, "extends", &["{"]);
                if let Some(clause) = extends_clause {
                    for target_name in parse_type_identifier_list(&clause) {
                        let Some((target_symbol_id, confidence)) = resolve_related_symbol_id(
                            &file.path,
                            &target_name,
                            Some("Interface"),
                            &symbols_by_file_name,
                            &imports_by_source,
                        ) else {
                            continue;
                        };

                        if target_symbol_id == source_symbol_id {
                            continue;
                        }

                        let key = format!("EXTENDS:{}->{target_symbol_id}", source_symbol_id);
                        if !seen.insert(key) {
                            continue;
                        }

                        heritages.push(ParsedHeritage {
                            source_symbol_id: source_symbol_id.clone(),
                            target_symbol_id,
                            rel_type: "EXTENDS".to_string(),
                            confidence,
                        });
                    }
                }
            }
        }
    }

    heritages.sort_by(|a, b| {
        a.rel_type
            .cmp(&b.rel_type)
            .then(a.source_symbol_id.cmp(&b.source_symbol_id))
            .then(a.target_symbol_id.cmp(&b.target_symbol_id))
    });
    Ok(heritages)
}

fn strip_inline_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => line.get(..idx).unwrap_or(""),
        None => line,
    }
}

fn extract_clause(line: &str, keyword: &str, stop_keywords: &[&str]) -> Option<String> {
    let needle = format!("{keyword} ");
    let start = line.find(&needle)?;
    let rest = line.get(start + needle.len()..)?;

    let mut end = rest.len();
    for stop in stop_keywords {
        if let Some(idx) = rest.find(stop) {
            end = end.min(idx);
        }
    }

    let clause = rest.get(..end)?.trim();
    if clause.is_empty() {
        None
    } else {
        Some(clause.to_string())
    }
}

fn parse_type_identifier_list(clause: &str) -> Vec<String> {
    clause
        .split(',')
        .filter_map(extract_type_identifier)
        .collect()
}

fn extract_type_identifier(raw: &str) -> Option<String> {
    let mut part = raw.trim();
    if part.is_empty() {
        return None;
    }

    if let Some((head, _)) = part.split_once('<') {
        part = head.trim();
    }
    if let Some((head, _)) = part.split_once('{') {
        part = head.trim();
    }
    if let Some((head, _)) = part.split_once('(') {
        part = head.trim();
    }
    if let Some((head, _)) = part.split_once('=') {
        part = head.trim();
    }

    let part = part.split_whitespace().last().unwrap_or_default();
    let part = part.rsplit('.').next().unwrap_or_default().trim();
    if part.is_empty() {
        return None;
    }

    let mut ident = String::new();
    for ch in part.chars() {
        if is_identifier_continue(ch) {
            ident.push(ch);
        } else {
            break;
        }
    }

    if ident.is_empty() { None } else { Some(ident) }
}

fn looks_like_definition_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("function ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("interface ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("type ")
}

fn find_caller_symbol<'a>(
    symbols: &'a [&'a ParsedSymbol],
    line_no: u64,
) -> Option<&'a ParsedSymbol> {
    let mut current: Option<&ParsedSymbol> = None;
    for symbol in symbols {
        if symbol.start_line <= line_no && matches!(symbol.label.as_str(), "Function" | "Class") {
            current = Some(symbol);
        } else if symbol.start_line > line_no {
            break;
        }
    }
    current
}

fn resolve_callee_symbol_id(
    source_file: &str,
    callee_name: &str,
    symbols_by_file_name: &BTreeMap<String, BTreeMap<String, Vec<&ParsedSymbol>>>,
    imports_by_source: &BTreeMap<String, Vec<String>>,
) -> Option<(String, f32)> {
    if let Some(same_file) = symbols_by_file_name
        .get(source_file)
        .and_then(|m| m.get(callee_name))
        .and_then(|symbols| symbols.first())
    {
        return Some((same_file.id.clone(), 0.9));
    }

    if let Some(import_targets) = imports_by_source.get(source_file) {
        for target_file in import_targets {
            if let Some(imported) = symbols_by_file_name
                .get(target_file)
                .and_then(|m| m.get(callee_name))
                .and_then(|symbols| symbols.first())
            {
                return Some((imported.id.clone(), 0.7));
            }
        }
    }

    None
}

fn resolve_symbol_id_in_file(
    file_path: &str,
    symbol_name: &str,
    preferred_label: Option<&str>,
    symbols_by_file_name: &BTreeMap<String, BTreeMap<String, Vec<&ParsedSymbol>>>,
) -> Option<String> {
    let candidates = symbols_by_file_name.get(file_path)?.get(symbol_name)?;
    if let Some(label) = preferred_label {
        if let Some(symbol) = candidates.iter().find(|s| s.label == label) {
            return Some(symbol.id.clone());
        }
    }
    candidates.first().map(|s| s.id.clone())
}

fn resolve_related_symbol_id(
    source_file: &str,
    symbol_name: &str,
    preferred_label: Option<&str>,
    symbols_by_file_name: &BTreeMap<String, BTreeMap<String, Vec<&ParsedSymbol>>>,
    imports_by_source: &BTreeMap<String, Vec<String>>,
) -> Option<(String, f32)> {
    if let Some(id) = resolve_symbol_id_in_file(
        source_file,
        symbol_name,
        preferred_label,
        symbols_by_file_name,
    ) {
        return Some((id, 0.85));
    }

    if let Some(import_targets) = imports_by_source.get(source_file) {
        for target_file in import_targets {
            if let Some(id) = resolve_symbol_id_in_file(
                target_file,
                symbol_name,
                preferred_label,
                symbols_by_file_name,
            ) {
                return Some((id, 0.7));
            }
        }
    }

    None
}

fn extract_call_identifiers(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut out = Vec::<String>::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if !is_identifier_start(ch) {
            i += 1;
            continue;
        }

        let start = i;
        i += 1;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if is_identifier_continue(c) {
                i += 1;
            } else {
                break;
            }
        }

        let mut j = i;
        while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] as char != '(' {
            continue;
        }

        let ident = line.get(start..i).unwrap_or_default();
        if ident.is_empty() {
            continue;
        }

        let prev = prev_non_whitespace_char(line, start);
        if matches!(prev, Some('.') | Some(':')) {
            continue;
        }

        out.push(ident.to_string());
    }

    out
}

fn prev_non_whitespace_char(line: &str, idx: usize) -> Option<char> {
    let prefix = line.get(..idx)?;
    prefix.chars().rev().find(|ch| !ch.is_whitespace())
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

fn is_non_call_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "for"
            | "while"
            | "switch"
            | "catch"
            | "return"
            | "typeof"
            | "delete"
            | "void"
            | "import"
            | "export"
            | "new"
            | "super"
            | "console"
    )
}

fn build_process_summaries(graph: &StructureGraph) -> Vec<HeuristicProcess> {
    let mut out_neighbors = BTreeMap::<String, HashSet<String>>::new();
    let mut undirected = BTreeMap::<String, HashSet<String>>::new();
    let mut indegree = BTreeMap::<String, u64>::new();

    for rel in &graph.relationships {
        if rel.rel_type != "CALLS" {
            continue;
        }

        out_neighbors
            .entry(rel.source_id.clone())
            .or_default()
            .insert(rel.target_id.clone());
        indegree
            .entry(rel.target_id.clone())
            .and_modify(|d| *d += 1)
            .or_insert(1);
        indegree.entry(rel.source_id.clone()).or_insert(0);

        undirected
            .entry(rel.source_id.clone())
            .or_default()
            .insert(rel.target_id.clone());
        undirected
            .entry(rel.target_id.clone())
            .or_default()
            .insert(rel.source_id.clone());
    }

    if undirected.is_empty() {
        return Vec::new();
    }

    let mut visited = HashSet::<String>::new();
    let mut processes = Vec::<HeuristicProcess>::new();
    let mut process_index = 0usize;

    for node in undirected.keys() {
        if visited.contains(node) {
            continue;
        }

        let mut component_nodes = HashSet::<String>::new();
        let mut stack = vec![node.clone()];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            component_nodes.insert(cur.clone());
            if let Some(next) = undirected.get(&cur) {
                for n in next {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }

        let mut entry_candidates: Vec<String> = component_nodes
            .iter()
            .filter(|id| indegree.get(*id).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        if entry_candidates.is_empty() {
            entry_candidates = component_nodes.iter().cloned().collect();
        }
        entry_candidates.sort();
        let entry = entry_candidates
            .into_iter()
            .next()
            .unwrap_or_else(|| node.clone());

        let (symbol_ids, max_depth) = traverse_process(&entry, &component_nodes, &out_neighbors);
        process_index += 1;
        processes.push(HeuristicProcess {
            id: format!("process_{process_index}"),
            entry_symbol_id: entry,
            symbol_count: symbol_ids.len() as u64,
            step_count: max_depth.saturating_add(1),
            symbols: symbol_ids,
        });
    }

    processes
}

fn traverse_process(
    entry: &str,
    component_nodes: &HashSet<String>,
    out_neighbors: &BTreeMap<String, HashSet<String>>,
) -> (Vec<String>, u64) {
    let mut visited = HashSet::<String>::new();
    let mut stack = vec![(entry.to_string(), 0u64)];
    let mut max_depth = 0u64;

    while let Some((cur, depth)) = stack.pop() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        max_depth = max_depth.max(depth);
        if let Some(next) = out_neighbors.get(&cur) {
            for n in next {
                if component_nodes.contains(n) && !visited.contains(n) {
                    stack.push((n.clone(), depth + 1));
                }
            }
        }
    }

    for node in component_nodes {
        if !visited.contains(node) {
            visited.insert(node.clone());
        }
    }

    let mut symbols = visited.into_iter().collect::<Vec<_>>();
    symbols.sort();
    (symbols, max_depth)
}

fn build_community_summaries(graph: &StructureGraph) -> Vec<HeuristicCommunity> {
    let mut node_by_id = BTreeMap::<String, &StructureNode>::new();
    for node in &graph.nodes {
        node_by_id.insert(node.id.clone(), node);
    }

    let mut adjacency = BTreeMap::<String, HashSet<String>>::new();
    let mut active_files = HashSet::<String>::new();

    for rel in &graph.relationships {
        match rel.rel_type.as_str() {
            "IMPORTS" => {
                if is_file_id(&rel.source_id) && is_file_id(&rel.target_id) {
                    connect_undirected(&mut adjacency, &rel.source_id, &rel.target_id);
                    active_files.insert(rel.source_id.clone());
                    active_files.insert(rel.target_id.clone());
                }
            }
            "CALLS" | "EXTENDS" | "IMPLEMENTS" => {
                let Some(source_node) = node_by_id.get(&rel.source_id) else {
                    continue;
                };
                let Some(target_node) = node_by_id.get(&rel.target_id) else {
                    continue;
                };

                let source_file_id = generate_id("File", &source_node.file_path);
                let target_file_id = generate_id("File", &target_node.file_path);
                if source_file_id == target_file_id {
                    active_files.insert(source_file_id);
                    continue;
                }

                connect_undirected(&mut adjacency, &source_file_id, &target_file_id);
                active_files.insert(source_file_id);
                active_files.insert(target_file_id);
            }
            _ => {}
        }
    }

    if active_files.is_empty() {
        return build_fallback_communities(graph);
    }

    let mut visited = HashSet::<String>::new();
    let mut communities = Vec::<HeuristicCommunity>::new();
    let mut community_index = 0usize;
    for file in &active_files {
        if visited.contains(file) {
            continue;
        }

        let mut files = Vec::<String>::new();
        let mut stack = vec![file.clone()];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            files.push(strip_file_node_prefix(&cur).to_string());
            if let Some(next) = adjacency.get(&cur) {
                for n in next {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }

        files.sort();
        community_index += 1;
        communities.push(HeuristicCommunity {
            id: format!("community_{community_index}"),
            file_count: files.len() as u64,
            files,
        });
    }

    communities
}

fn build_fallback_communities(graph: &StructureGraph) -> Vec<HeuristicCommunity> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for node in &graph.nodes {
        if node.label != "File" {
            continue;
        }

        let group = node
            .file_path
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if group.is_empty() {
            continue;
        }
        groups
            .entry(group)
            .or_default()
            .push(node.file_path.clone());
    }

    let mut communities = Vec::<HeuristicCommunity>::new();
    for (idx, (_group, mut files)) in groups.into_iter().enumerate() {
        files.sort();
        communities.push(HeuristicCommunity {
            id: format!("community_{}", idx + 1),
            file_count: files.len() as u64,
            files,
        });
    }

    communities
}

fn is_file_id(node_id: &str) -> bool {
    node_id.starts_with("File:")
}

fn strip_file_node_prefix(node_id: &str) -> &str {
    node_id.strip_prefix("File:").unwrap_or(node_id)
}

fn connect_undirected(adjacency: &mut BTreeMap<String, HashSet<String>>, a: &str, b: &str) {
    adjacency
        .entry(a.to_string())
        .or_default()
        .insert(b.to_string());
    adjacency
        .entry(b.to_string())
        .or_default()
        .insert(a.to_string());
}

fn extract_import_specifiers(content: &str, language: SupportedLanguage) -> Vec<String> {
    let mut specifiers = Vec::<String>::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        match language {
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
                if let Some(spec) = extract_js_ts_import(line) {
                    specifiers.push(spec);
                }
            }
            _ => {}
        }
    }

    specifiers
}

fn extract_js_ts_import(line: &str) -> Option<String> {
    if line.starts_with("import ") || line.starts_with("export ") {
        if line.contains(" from ") {
            return extract_quoted_literal(line);
        }

        if line.starts_with("import ") {
            return extract_quoted_literal(line);
        }
    }

    if line.contains("require(") {
        let start = line.find("require(")?;
        let inner = line.get(start + "require(".len()..)?;
        return extract_quoted_literal(inner);
    }

    None
}

fn extract_quoted_literal(text: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        if let Some(start) = text.find(quote) {
            let rest = text.get(start + 1..)?;
            if let Some(end) = rest.find(quote) {
                let candidate = rest.get(..end)?.trim();
                if !candidate.is_empty() {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

fn resolve_relative_import_path(
    source_file: &str,
    specifier: &str,
    language: SupportedLanguage,
    file_set: &HashSet<String>,
) -> Option<String> {
    let base_dir = Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let import_base = normalize_relative_path(base_dir.join(specifier));
    if import_base.is_empty() {
        return None;
    }

    if file_set.contains(&import_base) {
        return Some(import_base);
    }

    let ext_candidates = match language {
        SupportedLanguage::TypeScript => {
            vec![".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"]
        }
        SupportedLanguage::JavaScript => {
            vec![".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"]
        }
        _ => vec![],
    };

    for ext in &ext_candidates {
        let candidate = format!("{import_base}{ext}");
        if file_set.contains(&candidate) {
            return Some(candidate);
        }
    }

    for ext in &ext_candidates {
        let candidate = format!("{import_base}/index{ext}");
        if file_set.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn normalize_relative_path(path: std::path::PathBuf) -> String {
    use std::path::Component;

    let mut parts = Vec::<String>::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => {
                parts.push(part.to_string_lossy().to_string());
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }

    parts.join("/")
}

fn extract_symbols_from_content(
    content: &str,
    file_path: &str,
    language: SupportedLanguage,
) -> Vec<ParsedSymbol> {
    let mut out = Vec::<ParsedSymbol>::new();

    for (idx, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let line_no = (idx + 1) as u64;

        match language {
            SupportedLanguage::Rust => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "fn ", "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "struct ", "Struct",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "trait ", "Trait",
                );
            }
            SupportedLanguage::TypeScript => {
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "function ",
                    "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "interface ",
                    "Interface",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
            SupportedLanguage::JavaScript => {
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "function ",
                    "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
            }
            SupportedLanguage::Python => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "def ", "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
            }
            SupportedLanguage::Java => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "interface ",
                    "Interface",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
            SupportedLanguage::C => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "struct ", "Struct",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
            SupportedLanguage::Cpp => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "struct ", "Struct",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
            SupportedLanguage::CSharp => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "interface ",
                    "Interface",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
            SupportedLanguage::Go => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "func ", "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "type ", "Type",
                );
            }
            SupportedLanguage::PHP => {
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "function ",
                    "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "interface ",
                    "Interface",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "trait ", "Trait",
                );
            }
            SupportedLanguage::Kotlin => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "fun ", "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "interface ",
                    "Interface",
                );
            }
            SupportedLanguage::Swift => {
                push_if_match(
                    &mut out, line, file_path, language, line_no, "func ", "Function",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "class ", "Class",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "struct ", "Struct",
                );
                push_if_match(
                    &mut out,
                    line,
                    file_path,
                    language,
                    line_no,
                    "protocol ",
                    "Interface",
                );
                push_if_match(
                    &mut out, line, file_path, language, line_no, "enum ", "Enum",
                );
            }
        }
    }

    out
}

fn push_if_match(
    out: &mut Vec<ParsedSymbol>,
    line: &str,
    file_path: &str,
    language: SupportedLanguage,
    line_no: u64,
    keyword: &str,
    label: &str,
) {
    let Some(name) = extract_identifier_after_keyword(line, keyword) else {
        return;
    };

    let id = format!("{label}:{file_path}:{name}:{line_no}");
    out.push(ParsedSymbol {
        id,
        label: label.to_string(),
        name,
        file_path: file_path.to_string(),
        start_line: line_no,
        language,
    });
}

fn extract_identifier_after_keyword(line: &str, keyword: &str) -> Option<String> {
    let idx = line.find(keyword)?;
    if idx > 0 {
        let prev = line.as_bytes().get(idx - 1).copied().unwrap_or_default() as char;
        if prev.is_ascii_alphanumeric() || prev == '_' {
            return None;
        }
    }

    let start = idx + keyword.len();
    let rest = line.get(start..)?.trim_start();
    let mut ident = String::new();

    for ch in rest.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            ident.push(ch);
        } else {
            break;
        }
    }

    if ident.is_empty() { None } else { Some(ident) }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        MAX_FILE_SIZE_BYTES, ScannedFile, SupportedLanguage, build_structure_graph,
        detect_language_from_filename, extract_identifier_after_keyword,
        extract_symbols_from_content, parse_calls, parse_heritage, parse_imports,
        run_ingestion_pipeline, scan_repository_paths, should_ignore_path,
    };

    #[test]
    fn detect_language_matches_supported_extensions() {
        assert_eq!(
            detect_language_from_filename("src/main.tsx"),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            detect_language_from_filename("src/server.js"),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            detect_language_from_filename("src/main.rs"),
            Some(SupportedLanguage::Rust)
        );
        assert_eq!(
            detect_language_from_filename("src/app.kt"),
            Some(SupportedLanguage::Kotlin)
        );
        assert_eq!(detect_language_from_filename("README.md"), None);
    }

    #[test]
    fn ignore_path_filters_expected_files() {
        assert!(should_ignore_path("node_modules/react/index.js"));
        assert!(should_ignore_path("src/types/global.d.ts"));
        assert!(should_ignore_path("assets/logo.png"));
        assert!(!should_ignore_path("src/main.rs"));
    }

    #[test]
    fn build_structure_graph_creates_unique_nodes_and_edges() {
        let files = vec![
            ScannedFile {
                path: "src/main.rs".to_string(),
                size_bytes: 10,
                language: Some(SupportedLanguage::Rust),
            },
            ScannedFile {
                path: "src/lib.rs".to_string(),
                size_bytes: 10,
                language: Some(SupportedLanguage::Rust),
            },
            ScannedFile {
                path: "tests/basic.rs".to_string(),
                size_bytes: 10,
                language: Some(SupportedLanguage::Rust),
            },
        ];

        let graph = build_structure_graph(&files, &[], &[], &[], &[]);

        assert_eq!(graph.nodes.len(), 5);
        assert_eq!(graph.relationships.len(), 3);
        assert!(graph.nodes.iter().any(|n| n.id == "Folder:src"));
        assert!(graph.nodes.iter().any(|n| n.id == "File:src/main.rs"));
        assert!(
            graph
                .relationships
                .iter()
                .any(|r| r.id == "CONTAINS:Folder:src->File:src/main.rs")
        );
    }

    #[test]
    fn build_structure_graph_adds_define_edges_for_symbols() {
        let files = vec![ScannedFile {
            path: "src/main.rs".to_string(),
            size_bytes: 10,
            language: Some(SupportedLanguage::Rust),
        }];

        let symbols = extract_symbols_from_content(
            "pub fn main() {}\nstruct Service {}\n",
            "src/main.rs",
            SupportedLanguage::Rust,
        );
        let graph = build_structure_graph(&files, &symbols, &[], &[], &[]);

        assert!(graph.nodes.iter().any(|n| n.label == "Function"));
        assert!(graph.nodes.iter().any(|n| n.label == "Struct"));
        assert!(
            graph
                .relationships
                .iter()
                .any(|r| r.rel_type == "DEFINES" && r.source_id == "File:src/main.rs")
        );
    }

    #[test]
    fn parse_imports_and_graph_add_import_edges() {
        let root = create_temp_dir("gitnexus_rs_import_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(root.join("src/utils.ts"), "export function helper() {}\n")
            .expect("failed to write utils.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { helper } from './utils';\nhelper();\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(
            imports
                .iter()
                .any(|i| i.source_file == "src/main.ts" && i.target_file == "src/utils.ts")
        );

        let graph = build_structure_graph(&scan.files, &[], &imports, &[], &[]);
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "IMPORTS"
                && r.source_id == "File:src/main.ts"
                && r.target_id == "File:src/utils.ts"
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_calls_detects_local_and_imported_calls() {
        let root = create_temp_dir("gitnexus_rs_calls_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(root.join("src/utils.ts"), "export function helper() {}\n")
            .expect("failed to write utils.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { helper } from './utils';\nfunction run() {\n  helper();\n  local();\n}\nfunction local() {}\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let symbols = scan
            .files
            .iter()
            .flat_map(|f| {
                let content = fs::read_to_string(root.join(&f.path)).unwrap_or_default();
                let lang = f.language.unwrap_or(SupportedLanguage::TypeScript);
                extract_symbols_from_content(&content, &f.path, lang)
            })
            .collect::<Vec<_>>();
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");
        let calls =
            parse_calls(&root, &scan.files, &symbols, &imports).expect("calls should parse");

        let run_id = symbols
            .iter()
            .find(|s| s.file_path == "src/main.ts" && s.name == "run")
            .expect("run symbol should exist")
            .id
            .clone();
        let local_id = symbols
            .iter()
            .find(|s| s.file_path == "src/main.ts" && s.name == "local")
            .expect("local symbol should exist")
            .id
            .clone();
        let helper_id = symbols
            .iter()
            .find(|s| s.file_path == "src/utils.ts" && s.name == "helper")
            .expect("helper symbol should exist")
            .id
            .clone();

        assert!(
            calls
                .iter()
                .any(|c| c.source_symbol_id == run_id && c.target_symbol_id == local_id)
        );
        assert!(
            calls
                .iter()
                .any(|c| c.source_symbol_id == run_id && c.target_symbol_id == helper_id)
        );

        let graph = build_structure_graph(&scan.files, &symbols, &imports, &calls, &[]);
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "CALLS"
                && r.source_id == run_id
                && (r.target_id == local_id || r.target_id == helper_id)
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_heritage_detects_extends_and_implements() {
        let root = create_temp_dir("gitnexus_rs_heritage_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(
            root.join("src/base.ts"),
            "export class Base {}\nexport interface Worker {}\n",
        )
        .expect("failed to write base.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { Base, Worker } from './base';\ninterface AdvancedWorker extends Worker {}\nclass Service extends Base implements Worker {}\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let symbols = scan
            .files
            .iter()
            .flat_map(|f| {
                let content = fs::read_to_string(root.join(&f.path)).unwrap_or_default();
                let lang = f.language.unwrap_or(SupportedLanguage::TypeScript);
                extract_symbols_from_content(&content, &f.path, lang)
            })
            .collect::<Vec<_>>();
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");
        let heritages =
            parse_heritage(&root, &scan.files, &symbols, &imports).expect("heritage should parse");

        let service_id = symbols
            .iter()
            .find(|s| s.file_path == "src/main.ts" && s.name == "Service")
            .expect("service symbol should exist")
            .id
            .clone();
        let base_id = symbols
            .iter()
            .find(|s| s.file_path == "src/base.ts" && s.name == "Base")
            .expect("base symbol should exist")
            .id
            .clone();
        let worker_id = symbols
            .iter()
            .find(|s| s.file_path == "src/base.ts" && s.name == "Worker")
            .expect("worker symbol should exist")
            .id
            .clone();
        let advanced_worker_id = symbols
            .iter()
            .find(|s| s.file_path == "src/main.ts" && s.name == "AdvancedWorker")
            .expect("advanced worker symbol should exist")
            .id
            .clone();

        assert!(heritages.iter().any(|h| {
            h.rel_type == "EXTENDS"
                && h.source_symbol_id == service_id
                && h.target_symbol_id == base_id
        }));
        assert!(heritages.iter().any(|h| {
            h.rel_type == "IMPLEMENTS"
                && h.source_symbol_id == service_id
                && h.target_symbol_id == worker_id
        }));
        assert!(heritages.iter().any(|h| {
            h.rel_type == "EXTENDS"
                && h.source_symbol_id == advanced_worker_id
                && h.target_symbol_id == worker_id
        }));

        let graph = build_structure_graph(&scan.files, &symbols, &imports, &[], &heritages);
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "EXTENDS" && r.source_id == service_id && r.target_id == base_id
        }));
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "IMPLEMENTS" && r.source_id == service_id && r.target_id == worker_id
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn run_ingestion_pipeline_populates_process_and_community_stats() {
        let root = create_temp_dir("gitnexus_rs_stats_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(
            root.join("src/base.ts"),
            "export class Base {}\nexport interface Worker {}\n",
        )
        .expect("failed to write base.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { Base, Worker } from './base';\nclass Service extends Base implements Worker {\n  run() {\n    helper();\n  }\n}\nfunction helper() {}\n",
        )
        .expect("failed to write main.ts");

        let result = run_ingestion_pipeline(&root).expect("pipeline should succeed");
        assert!(result.stats.communities >= 1);
        assert!(result.stats.processes >= 1);

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn scan_repository_paths_skips_ignored_and_large_files() {
        let root = create_temp_dir("gitnexus_rs_ingestion_test");

        fs::create_dir_all(root.join("src")).expect("failed to create src");
        fs::create_dir_all(root.join("node_modules/pkg")).expect("failed to create node_modules");

        fs::write(root.join("src/main.rs"), "fn main() {}").expect("failed to write main.rs");
        fs::write(
            root.join("node_modules/pkg/index.js"),
            "module.exports = {}",
        )
        .expect("failed to write node_modules file");

        let large = vec![b'a'; (MAX_FILE_SIZE_BYTES as usize) + 1];
        fs::write(root.join("src/huge.ts"), large).expect("failed to write large file");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let paths: Vec<String> = scan.files.iter().map(|f| f.path.clone()).collect();

        assert_eq!(scan.skipped_large_files, 1);
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.contains(&"src/huge.ts".to_string()));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn extract_identifier_parses_symbol_name() {
        assert_eq!(
            extract_identifier_after_keyword("pub fn run(value: i32) {}", "fn "),
            Some("run".to_string())
        );
        assert_eq!(
            extract_identifier_after_keyword("export class Service {}", "class "),
            Some("Service".to_string())
        );
        assert_eq!(
            extract_identifier_after_keyword("somethingfunction bad()", "function "),
            None
        );
    }

    #[test]
    fn extract_symbols_handles_multiple_languages() {
        let rust = extract_symbols_from_content(
            "pub struct User {}\nfn run() {}\ntrait Worker {}\n",
            "src/lib.rs",
            SupportedLanguage::Rust,
        );
        assert!(rust.iter().any(|s| s.label == "Struct" && s.name == "User"));
        assert!(
            rust.iter()
                .any(|s| s.label == "Function" && s.name == "run")
        );
        assert!(
            rust.iter()
                .any(|s| s.label == "Trait" && s.name == "Worker")
        );

        let ts = extract_symbols_from_content(
            "export interface Client {}\nclass Api {}\nfunction call() {}\n",
            "src/api.ts",
            SupportedLanguage::TypeScript,
        );
        assert!(
            ts.iter()
                .any(|s| s.label == "Interface" && s.name == "Client")
        );
        assert!(ts.iter().any(|s| s.label == "Class" && s.name == "Api"));
        assert!(ts.iter().any(|s| s.label == "Function" && s.name == "call"));

        let py = extract_symbols_from_content(
            "class Worker:\n    pass\n\ndef run():\n    return 1\n",
            "app.py",
            SupportedLanguage::Python,
        );
        assert!(py.iter().any(|s| s.label == "Class" && s.name == "Worker"));
        assert!(py.iter().any(|s| s.label == "Function" && s.name == "run"));
    }

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        dir.push(format!("{prefix}_{}_{}", std::process::id(), nonce));
        fs::create_dir_all(&dir).expect("failed to create temp directory");
        dir
    }
}
