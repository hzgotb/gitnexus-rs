use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

mod graph;
mod parser_loader;
mod relations;
mod scan;
mod summaries;
mod symbols;

const MAX_FILE_SIZE_BYTES: u64 = 512 * 1024;
const PARSE_CHUNK_MAX_BYTES: u64 = 4 * 1024 * 1024;
const PARSE_CHUNK_MAX_FILES: usize = 64;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParseChunk {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionStage {
    Symbols,
    Imports,
    Calls,
    Heritage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestionProgress {
    pub stage: IngestionStage,
    pub completed_chunks: u64,
    pub total_chunks: u64,
    pub completed_files: u64,
    pub total_files: u64,
}

pub fn run_ingestion_pipeline(repo_path: &Path) -> Result<IngestionResult> {
    run_ingestion_pipeline_with_progress(repo_path, |_| {})
}

pub fn run_ingestion_pipeline_with_progress<F>(
    repo_path: &Path,
    mut on_progress: F,
) -> Result<IngestionResult>
where
    F: FnMut(IngestionProgress),
{
    let scan = scan_repository_paths(repo_path)?;
    let chunks = build_parse_chunks(&scan.files);
    let symbols = parse_symbols_in_chunks(repo_path, &scan.files, &chunks, &mut on_progress)?;
    let imports = parse_imports_in_chunks(repo_path, &scan.files, &chunks, &mut on_progress)?;
    let calls = parse_calls_in_chunks(
        repo_path,
        &scan.files,
        &chunks,
        &symbols,
        &imports,
        &mut on_progress,
    )?;
    let heritages = parse_heritage_in_chunks(
        repo_path,
        &scan.files,
        &chunks,
        &symbols,
        &imports,
        &mut on_progress,
    )?;
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

fn build_parse_chunks(files: &[ScannedFile]) -> Vec<ParseChunk> {
    build_parse_chunks_with_limits(files, PARSE_CHUNK_MAX_BYTES, PARSE_CHUNK_MAX_FILES)
}

fn build_parse_chunks_with_limits(
    files: &[ScannedFile],
    max_bytes: u64,
    max_files: usize,
) -> Vec<ParseChunk> {
    if files.is_empty() {
        return Vec::new();
    }

    let byte_budget = max_bytes.max(1);
    let file_budget = max_files.max(1);
    let mut chunks = Vec::<ParseChunk>::new();

    let mut start = 0usize;
    let mut current_bytes = 0u64;
    let mut current_files = 0usize;

    for (idx, file) in files.iter().enumerate() {
        let file_bytes = file.size_bytes.max(1);
        let exceed_budget = current_files > 0
            && (current_bytes.saturating_add(file_bytes) > byte_budget
                || current_files >= file_budget);
        if exceed_budget {
            chunks.push(ParseChunk { start, end: idx });
            start = idx;
            current_bytes = 0;
            current_files = 0;
        }

        current_bytes = current_bytes.saturating_add(file_bytes);
        current_files += 1;
    }

    if start < files.len() {
        chunks.push(ParseChunk {
            start,
            end: files.len(),
        });
    }

    chunks
}

fn emit_ingestion_progress<F>(
    on_progress: &mut F,
    stage: IngestionStage,
    completed_chunks: u64,
    total_chunks: u64,
    completed_files: u64,
    total_files: u64,
) where
    F: FnMut(IngestionProgress),
{
    on_progress(IngestionProgress {
        stage,
        completed_chunks,
        total_chunks,
        completed_files,
        total_files,
    });
}

fn worker_pool_size(total_chunks: usize) -> usize {
    if total_chunks == 0 {
        return 1;
    }

    let available = thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    available.min(total_chunks).max(1)
}

fn result_channel_capacity(worker_count: usize) -> usize {
    worker_count.saturating_mul(2).max(1)
}

fn run_chunk_stage_with_worker_pool<T, F>(
    stage: IngestionStage,
    files: &[ScannedFile],
    chunks: &[ParseChunk],
    on_progress: &mut F,
    parse_chunk: impl Fn(&[ScannedFile]) -> Result<Vec<T>> + Sync,
) -> Result<Vec<T>>
where
    T: Send,
    F: FnMut(IngestionProgress),
{
    if chunks.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = worker_pool_size(chunks.len());
    let stop_dispatch = AtomicBool::new(false);
    let next_chunk = AtomicUsize::new(0);
    let (tx, rx) =
        mpsc::sync_channel::<(u64, Result<Vec<T>>)>(result_channel_capacity(worker_count));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = tx.clone();
            let parse_chunk = &parse_chunk;
            let next_chunk = &next_chunk;
            let stop_dispatch = &stop_dispatch;
            scope.spawn(move || {
                loop {
                    if stop_dispatch.load(Ordering::Acquire) {
                        break;
                    }

                    let idx = next_chunk.fetch_add(1, Ordering::Relaxed);
                    if idx >= chunks.len() {
                        break;
                    }

                    if stop_dispatch.load(Ordering::Acquire) {
                        break;
                    }

                    let chunk = chunks[idx];
                    let chunk_files = &files[chunk.start..chunk.end];
                    let result = parse_chunk(chunk_files);

                    if result.is_err() {
                        stop_dispatch.store(true, Ordering::Release);
                    }

                    if sender.send((chunk_files.len() as u64, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut all_items = Vec::<T>::new();
        let mut first_error: Option<anyhow::Error> = None;
        let mut completed_chunks = 0u64;
        let mut completed_files = 0u64;
        while let Ok((file_count, chunk_result)) = rx.recv() {
            completed_chunks += 1;
            completed_files += file_count;
            emit_ingestion_progress(
                on_progress,
                stage,
                completed_chunks,
                chunks.len() as u64,
                completed_files,
                files.len() as u64,
            );

            match chunk_result {
                Ok(mut values) => {
                    if first_error.is_none() {
                        all_items.append(&mut values);
                    }
                }
                Err(err) => {
                    if first_error.is_none() {
                        stop_dispatch.store(true, Ordering::Release);
                        first_error = Some(err);
                    }
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        Ok(all_items)
    })
}

fn parse_symbols_in_chunks<F>(
    repo_path: &Path,
    files: &[ScannedFile],
    chunks: &[ParseChunk],
    on_progress: &mut F,
) -> Result<Vec<ParsedSymbol>>
where
    F: FnMut(IngestionProgress),
{
    let mut symbols = run_chunk_stage_with_worker_pool(
        IngestionStage::Symbols,
        files,
        chunks,
        on_progress,
        |chunk_files| parse_symbols(repo_path, chunk_files),
    )?;

    symbols.sort_by(|a, b| a.id.cmp(&b.id));
    symbols.dedup_by(|a, b| a.id == b.id);
    Ok(symbols)
}

fn parse_imports_in_chunks<F>(
    repo_path: &Path,
    files: &[ScannedFile],
    chunks: &[ParseChunk],
    on_progress: &mut F,
) -> Result<Vec<ParsedImport>>
where
    F: FnMut(IngestionProgress),
{
    let mut imports = run_chunk_stage_with_worker_pool(
        IngestionStage::Imports,
        files,
        chunks,
        on_progress,
        |chunk_files| parse_imports_for_sources(repo_path, files, chunk_files),
    )?;

    imports.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then(a.target_file.cmp(&b.target_file))
    });
    imports.dedup_by(|a, b| a.source_file == b.source_file && a.target_file == b.target_file);
    Ok(imports)
}

fn parse_calls_in_chunks<F>(
    repo_path: &Path,
    files: &[ScannedFile],
    chunks: &[ParseChunk],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
    on_progress: &mut F,
) -> Result<Vec<ParsedCall>>
where
    F: FnMut(IngestionProgress),
{
    let mut calls = run_chunk_stage_with_worker_pool(
        IngestionStage::Calls,
        files,
        chunks,
        on_progress,
        |chunk_files| parse_calls(repo_path, chunk_files, symbols, imports),
    )?;

    calls.sort_by(|a, b| {
        a.source_symbol_id
            .cmp(&b.source_symbol_id)
            .then(a.target_symbol_id.cmp(&b.target_symbol_id))
    });
    calls.dedup_by(|a, b| {
        a.source_symbol_id == b.source_symbol_id && a.target_symbol_id == b.target_symbol_id
    });
    Ok(calls)
}

fn parse_heritage_in_chunks<F>(
    repo_path: &Path,
    files: &[ScannedFile],
    chunks: &[ParseChunk],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
    on_progress: &mut F,
) -> Result<Vec<ParsedHeritage>>
where
    F: FnMut(IngestionProgress),
{
    let mut heritages = run_chunk_stage_with_worker_pool(
        IngestionStage::Heritage,
        files,
        chunks,
        on_progress,
        |chunk_files| parse_heritage(repo_path, chunk_files, symbols, imports),
    )?;

    heritages.sort_by(|a, b| {
        a.rel_type
            .cmp(&b.rel_type)
            .then(a.source_symbol_id.cmp(&b.source_symbol_id))
            .then(a.target_symbol_id.cmp(&b.target_symbol_id))
    });
    heritages.dedup_by(|a, b| {
        a.rel_type == b.rel_type
            && a.source_symbol_id == b.source_symbol_id
            && a.target_symbol_id == b.target_symbol_id
    });
    Ok(heritages)
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
    scan::scan_repository_paths(repo_path)
}

fn build_structure_graph(
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
    calls: &[ParsedCall],
    heritages: &[ParsedHeritage],
) -> StructureGraph {
    graph::build_structure_graph(files, symbols, imports, calls, heritages)
}

fn generate_id(label: &str, name: &str) -> String {
    format!("{label}:{name}")
}

fn parse_symbols(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedSymbol>> {
    symbols::parse_symbols(repo_path, files)
}

#[cfg(test)]
fn parse_imports(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedImport>> {
    relations::parse_imports(repo_path, files)
}

fn parse_imports_for_sources(
    repo_path: &Path,
    all_files: &[ScannedFile],
    source_files: &[ScannedFile],
) -> Result<Vec<ParsedImport>> {
    relations::parse_imports_for_sources(repo_path, all_files, source_files)
}

fn parse_calls(
    repo_path: &Path,
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
) -> Result<Vec<ParsedCall>> {
    relations::parse_calls(repo_path, files, symbols, imports)
}

fn parse_heritage(
    repo_path: &Path,
    files: &[ScannedFile],
    symbols: &[ParsedSymbol],
    imports: &[ParsedImport],
) -> Result<Vec<ParsedHeritage>> {
    relations::parse_heritage(repo_path, files, symbols, imports)
}

fn build_process_summaries(graph: &StructureGraph) -> Vec<HeuristicProcess> {
    summaries::build_process_summaries(graph)
}

fn build_community_summaries(graph: &StructureGraph) -> Vec<HeuristicCommunity> {
    summaries::build_community_summaries(graph)
}

fn extract_identifier_after_keyword(line: &str, keyword: &str) -> Option<String> {
    symbols::extract_identifier_after_keyword(line, keyword)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        IngestionStage, MAX_FILE_SIZE_BYTES, ScannedFile, SupportedLanguage,
        build_parse_chunks_with_limits, build_structure_graph, extract_identifier_after_keyword,
        parse_calls, parse_heritage, parse_imports, result_channel_capacity,
        run_chunk_stage_with_worker_pool, run_ingestion_pipeline,
        run_ingestion_pipeline_with_progress, scan, scan_repository_paths, symbols,
    };

    #[test]
    fn detect_language_matches_supported_extensions() {
        assert_eq!(
            scan::detect_language_from_filename("src/main.tsx"),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            scan::detect_language_from_filename("src/server.js"),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            scan::detect_language_from_filename("src/main.rs"),
            Some(SupportedLanguage::Rust)
        );
        assert_eq!(
            scan::detect_language_from_filename("src/app.kt"),
            Some(SupportedLanguage::Kotlin)
        );
        assert_eq!(scan::detect_language_from_filename("README.md"), None);
    }

    #[test]
    fn ignore_path_filters_expected_files() {
        assert!(scan::should_ignore_path("node_modules/react/index.js"));
        assert!(scan::should_ignore_path("src/types/global.d.ts"));
        assert!(scan::should_ignore_path("assets/logo.png"));
        assert!(!scan::should_ignore_path("src/main.rs"));
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

        let symbols = symbols::extract_symbols_from_content(
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
    fn parse_imports_resolves_tsconfig_path_aliases() {
        let root = create_temp_dir("gitnexus_rs_tsconfig_alias_test");
        fs::create_dir_all(root.join("src/lib")).expect("failed to create src/lib");

        fs::write(
            root.join("tsconfig.json"),
            "{\n  // alias config\n  \"compilerOptions\": {\n    \"baseUrl\": \".\",\n    \"paths\": {\n      \"@core/*\": [\"src/lib/*\"]\n    }\n  }\n}\n",
        )
        .expect("failed to write tsconfig.json");

        fs::write(root.join("src/lib/math.ts"), "export function add() {}\n")
            .expect("failed to write math.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { add } from '@core/math';\nadd();\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(
            imports
                .iter()
                .any(|i| i.source_file == "src/main.ts" && i.target_file == "src/lib/math.ts")
        );

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_imports_resolves_baseurl_absolute_specifiers() {
        let root = create_temp_dir("gitnexus_rs_baseurl_import_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(
            root.join("tsconfig.json"),
            "{\n  \"compilerOptions\": {\n    \"baseUrl\": \".\"\n  }\n}\n",
        )
        .expect("failed to write tsconfig.json");

        fs::write(root.join("src/utils.ts"), "export function helper() {}\n")
            .expect("failed to write utils.ts");
        fs::write(
            root.join("src/main.ts"),
            "import { helper } from 'src/utils';\nhelper();\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(
            imports
                .iter()
                .any(|i| i.source_file == "src/main.ts" && i.target_file == "src/utils.ts")
        );

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_imports_resolves_module_context_suffix_paths() {
        let root = create_temp_dir("gitnexus_rs_module_context_import_test");
        fs::create_dir_all(root.join("packages/core/lib")).expect("failed to create core/lib");
        fs::create_dir_all(root.join("apps/web/src")).expect("failed to create apps/web/src");

        fs::write(
            root.join("packages/core/lib/math.ts"),
            "export function multiply() {}\n",
        )
        .expect("failed to write math.ts");
        fs::write(
            root.join("apps/web/src/main.ts"),
            "import { multiply } from 'core/lib/math';\nmultiply();\n",
        )
        .expect("failed to write main.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(imports.iter().any(|i| {
            i.source_file == "apps/web/src/main.ts" && i.target_file == "packages/core/lib/math.ts"
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_imports_resolves_go_module_imports() {
        let root = create_temp_dir("gitnexus_rs_go_module_import_test");
        fs::create_dir_all(root.join("cmd/app")).expect("failed to create cmd/app");
        fs::create_dir_all(root.join("pkg/math")).expect("failed to create pkg/math");

        fs::write(root.join("go.mod"), "module github.com/acme/demo\n")
            .expect("failed to write go.mod");
        fs::write(
            root.join("pkg/math/math.go"),
            "package math\nfunc Add(a, b int) int { return a + b }\n",
        )
        .expect("failed to write math.go");
        fs::write(
            root.join("cmd/app/main.go"),
            "package main\nimport \"github.com/acme/demo/pkg/math\"\nfunc main() { _ = math.Add(1, 2) }\n",
        )
        .expect("failed to write main.go");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(imports.iter().any(|i| {
            i.source_file == "cmd/app/main.go" && i.target_file == "pkg/math/math.go"
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_imports_resolves_php_psr4_autoload_imports() {
        let root = create_temp_dir("gitnexus_rs_php_psr4_import_test");
        fs::create_dir_all(root.join("src/Services")).expect("failed to create src/Services");

        fs::write(
            root.join("composer.json"),
            "{\n  \"autoload\": {\n    \"psr-4\": {\n      \"App\\\\\\\\\": \"src/\"\n    }\n  }\n}\n",
        )
        .expect("failed to write composer.json");
        fs::write(
            root.join("src/Services/Greeter.php"),
            "<?php\nnamespace App\\Services;\nclass Greeter {}\n",
        )
        .expect("failed to write Greeter.php");
        fs::write(
            root.join("src/index.php"),
            "<?php\nuse App\\Services\\Greeter;\n$g = new Greeter();\n",
        )
        .expect("failed to write index.php");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(imports.iter().any(
            |i| i.source_file == "src/index.php" && i.target_file == "src/Services/Greeter.php"
        ));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_imports_resolves_csharp_root_namespace_imports() {
        let root = create_temp_dir("gitnexus_rs_csharp_namespace_import_test");
        fs::create_dir_all(root.join("app/Services")).expect("failed to create app/Services");

        fs::write(
            root.join("app/App.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n    <RootNamespace>MyCompany.Product</RootNamespace>\n  </PropertyGroup>\n</Project>\n",
        )
        .expect("failed to write App.csproj");
        fs::write(
            root.join("app/Services/Worker.cs"),
            "namespace MyCompany.Product.Services;\npublic class Worker {}\n",
        )
        .expect("failed to write Worker.cs");
        fs::write(
            root.join("app/Program.cs"),
            "using MyCompany.Product.Services;\npublic class Program { public static void Main() {} }\n",
        )
        .expect("failed to write Program.cs");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");

        assert!(imports.iter().any(|i| {
            i.source_file == "app/Program.cs" && i.target_file == "app/Services/Worker.cs"
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
                symbols::extract_symbols_from_content(&content, &f.path, lang)
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
    fn parse_calls_detects_laravel_route_controller_calls() {
        let root = create_temp_dir("gitnexus_rs_laravel_route_calls_test");
        fs::create_dir_all(root.join("routes")).expect("failed to create routes");
        fs::create_dir_all(root.join("app/Http/Controllers"))
            .expect("failed to create app/Http/Controllers");

        fs::write(
            root.join("composer.json"),
            "{\n  \"autoload\": {\n    \"psr-4\": {\n      \"App\\\\\\\\\": \"app/\"\n    }\n  }\n}\n",
        )
        .expect("failed to write composer.json");
        fs::write(
            root.join("app/Http/Controllers/UserController.php"),
            "<?php\nnamespace App\\Http\\Controllers;\nclass UserController {\n  public function index() {}\n}\n",
        )
        .expect("failed to write UserController.php");
        fs::write(
            root.join("routes/web.php"),
            "<?php\nuse App\\Http\\Controllers\\UserController;\nRoute::get('/users', [UserController::class, 'index']);\n",
        )
        .expect("failed to write routes/web.php");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let symbols = scan
            .files
            .iter()
            .flat_map(|f| {
                let content = fs::read_to_string(root.join(&f.path)).unwrap_or_default();
                let lang = f.language.unwrap_or(SupportedLanguage::TypeScript);
                symbols::extract_symbols_from_content(&content, &f.path, lang)
            })
            .collect::<Vec<_>>();
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");
        let calls =
            parse_calls(&root, &scan.files, &symbols, &imports).expect("calls should parse");

        let index_id = symbols
            .iter()
            .find(|s| s.file_path == "app/Http/Controllers/UserController.php" && s.name == "index")
            .expect("index method should exist")
            .id
            .clone();
        let route_file_id = "File:routes/web.php".to_string();

        assert!(
            calls
                .iter()
                .any(|c| c.source_symbol_id == route_file_id && c.target_symbol_id == index_id)
        );

        let graph = build_structure_graph(&scan.files, &symbols, &imports, &calls, &[]);
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "CALLS" && r.source_id == route_file_id && r.target_id == index_id
        }));

        fs::remove_dir_all(&root).expect("failed to clean temp directory");
    }

    #[test]
    fn parse_calls_detects_js_route_file_handler_calls() {
        let root = create_temp_dir("gitnexus_rs_js_route_calls_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        fs::write(
            root.join("src/handlers.ts"),
            "export function listUsers() {}\nexport function healthCheck() {}\n",
        )
        .expect("failed to write handlers.ts");
        fs::write(
            root.join("src/routes.ts"),
            "import { listUsers, healthCheck } from './handlers';\nconst router = createRouter();\nrouter.get('/users', listUsers);\nrouter.get('/health', [healthCheck]);\nrouter.use(listUsers);\n",
        )
        .expect("failed to write routes.ts");

        let scan = scan_repository_paths(&root).expect("scan should succeed");
        let symbols = scan
            .files
            .iter()
            .flat_map(|f| {
                let content = fs::read_to_string(root.join(&f.path)).unwrap_or_default();
                let lang = f.language.unwrap_or(SupportedLanguage::TypeScript);
                symbols::extract_symbols_from_content(&content, &f.path, lang)
            })
            .collect::<Vec<_>>();
        let imports = parse_imports(&root, &scan.files).expect("imports should parse");
        let calls =
            parse_calls(&root, &scan.files, &symbols, &imports).expect("calls should parse");

        let list_users_id = symbols
            .iter()
            .find(|s| s.file_path == "src/handlers.ts" && s.name == "listUsers")
            .expect("listUsers should exist")
            .id
            .clone();
        let health_check_id = symbols
            .iter()
            .find(|s| s.file_path == "src/handlers.ts" && s.name == "healthCheck")
            .expect("healthCheck should exist")
            .id
            .clone();
        let route_file_id = "File:src/routes.ts".to_string();

        assert!(calls.iter().any(|c| {
            c.source_symbol_id == route_file_id && c.target_symbol_id == list_users_id
        }));
        assert!(calls.iter().any(|c| {
            c.source_symbol_id == route_file_id && c.target_symbol_id == health_check_id
        }));

        let graph = build_structure_graph(&scan.files, &symbols, &imports, &calls, &[]);
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "CALLS" && r.source_id == route_file_id && r.target_id == list_users_id
        }));
        assert!(graph.relationships.iter().any(|r| {
            r.rel_type == "CALLS" && r.source_id == route_file_id && r.target_id == health_check_id
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
                symbols::extract_symbols_from_content(&content, &f.path, lang)
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
    fn build_parse_chunks_respects_byte_and_file_limits() {
        let files = vec![
            ScannedFile {
                path: "src/a.ts".to_string(),
                size_bytes: 5,
                language: Some(SupportedLanguage::TypeScript),
            },
            ScannedFile {
                path: "src/b.ts".to_string(),
                size_bytes: 5,
                language: Some(SupportedLanguage::TypeScript),
            },
            ScannedFile {
                path: "src/c.ts".to_string(),
                size_bytes: 5,
                language: Some(SupportedLanguage::TypeScript),
            },
            ScannedFile {
                path: "src/d.ts".to_string(),
                size_bytes: 5,
                language: Some(SupportedLanguage::TypeScript),
            },
        ];

        let chunks = build_parse_chunks_with_limits(&files, 10, 2);
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start, chunks[0].end), (0, 2));
        assert_eq!((chunks[1].start, chunks[1].end), (2, 4));
    }

    #[test]
    fn result_channel_capacity_scales_with_worker_count() {
        assert_eq!(result_channel_capacity(0), 1);
        assert_eq!(result_channel_capacity(1), 2);
        assert_eq!(result_channel_capacity(4), 8);
    }

    #[test]
    fn run_chunk_stage_returns_error_without_deadlock() {
        let files = (0..16)
            .map(|idx| ScannedFile {
                path: format!("src/file{idx:02}.ts"),
                size_bytes: 1,
                language: Some(SupportedLanguage::TypeScript),
            })
            .collect::<Vec<_>>();
        let chunks = build_parse_chunks_with_limits(&files, 1, 1);

        let mut progress_events = Vec::new();
        let result = run_chunk_stage_with_worker_pool(
            IngestionStage::Symbols,
            &files,
            &chunks,
            &mut |progress| progress_events.push(progress),
            |_chunk_files| -> anyhow::Result<Vec<String>> {
                Err(anyhow::anyhow!("simulated parse error"))
            },
        );

        assert!(result.is_err());
        assert!(!progress_events.is_empty());
        assert!(
            progress_events
                .iter()
                .all(|p| p.total_chunks == chunks.len() as u64)
        );
    }

    #[test]
    fn run_ingestion_pipeline_reports_progress_for_all_stages() {
        let root = create_temp_dir("gitnexus_rs_progress_test");
        fs::create_dir_all(root.join("src")).expect("failed to create src");

        for idx in 0..70 {
            fs::write(
                root.join(format!("src/mod{idx:02}.ts")),
                format!("export function mod_{idx}() {{ return {idx}; }}\n"),
            )
            .expect("failed to write module file");
        }
        fs::write(
            root.join("src/main.ts"),
            "import { mod_69 } from './mod69';\nfunction run() { mod_69(); }\n",
        )
        .expect("failed to write main.ts");

        let mut progress_events = Vec::new();
        let result = run_ingestion_pipeline_with_progress(&root, |progress| {
            progress_events.push(progress);
        })
        .expect("pipeline should succeed");

        assert!(result.graph.relationships.iter().any(|r| {
            r.rel_type == "IMPORTS"
                && r.source_id == "File:src/main.ts"
                && r.target_id == "File:src/mod69.ts"
        }));

        assert!(progress_events.iter().any(|p| p.total_chunks > 1));
        for stage in [
            IngestionStage::Symbols,
            IngestionStage::Imports,
            IngestionStage::Calls,
            IngestionStage::Heritage,
        ] {
            let stage_events = progress_events
                .iter()
                .copied()
                .filter(|progress| progress.stage == stage)
                .collect::<Vec<_>>();
            assert!(
                !stage_events.is_empty(),
                "expected progress events for stage {:?}",
                stage
            );

            let last = stage_events
                .last()
                .copied()
                .expect("stage should have a final progress event");
            assert_eq!(last.completed_chunks, last.total_chunks);
            assert_eq!(last.completed_files, last.total_files);

            for window in stage_events.windows(2) {
                let prev = window[0];
                let next = window[1];
                assert!(next.completed_chunks > prev.completed_chunks);
                assert!(next.completed_files >= prev.completed_files);
            }
        }

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
        let rust = symbols::extract_symbols_from_content(
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

        let ts = symbols::extract_symbols_from_content(
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

        let py = symbols::extract_symbols_from_content(
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
