use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use walkdir::WalkDir;

use super::{
    ParsedCall, ParsedHeritage, ParsedImport, ParsedSymbol, ScannedFile, SupportedLanguage,
    parser_loader,
};

#[derive(Debug, Clone)]
struct ImportResolutionContext {
    file_set: HashSet<String>,
    suffix_index: BTreeMap<String, String>,
    tsconfig_paths: Option<TsconfigPaths>,
    go_module_path: Option<String>,
    composer_psr4: Vec<ComposerPsr4Mapping>,
    csharp_projects: Vec<CSharpProjectConfig>,
}

#[derive(Debug, Clone)]
struct TsconfigPaths {
    aliases: Vec<(String, String)>,
    base_url: String,
}

#[derive(Debug, Clone)]
struct ComposerPsr4Mapping {
    namespace_prefix: String,
    directory_prefix: String,
}

#[derive(Debug, Clone)]
struct CSharpProjectConfig {
    project_dir: String,
    root_namespace: String,
}

#[derive(Debug, Deserialize)]
struct TsconfigFile {
    #[serde(rename = "compilerOptions")]
    compiler_options: Option<TsconfigCompilerOptions>,
}

#[derive(Debug, Deserialize)]
struct TsconfigCompilerOptions {
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    paths: Option<BTreeMap<String, Vec<String>>>,
}

fn build_import_resolution_context(
    repo_path: &Path,
    files: &[ScannedFile],
) -> ImportResolutionContext {
    let mut file_list = files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    file_list.sort();
    file_list.dedup();

    let file_set = file_list.iter().cloned().collect::<HashSet<_>>();
    let suffix_index = build_suffix_index(&file_list);
    let tsconfig_paths = load_tsconfig_paths(repo_path);
    let go_module_path = load_go_module_path(repo_path);
    let composer_psr4 = load_composer_psr4(repo_path);
    let csharp_projects = load_csharp_projects(repo_path);

    ImportResolutionContext {
        file_set,
        suffix_index,
        tsconfig_paths,
        go_module_path,
        composer_psr4,
        csharp_projects,
    }
}

#[cfg(test)]
pub(super) fn parse_imports(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedImport>> {
    parse_imports_for_sources(repo_path, files, files)
}

pub(super) fn parse_imports_for_sources(
    repo_path: &Path,
    all_files: &[ScannedFile],
    source_files: &[ScannedFile],
) -> Result<Vec<ParsedImport>> {
    let context = build_import_resolution_context(repo_path, all_files);
    let mut imports = Vec::<ParsedImport>::new();
    let mut seen = HashSet::<String>::new();

    for file in source_files {
        let Some(language) = file.language else {
            continue;
        };

        if !parser_loader::supports_import_extraction(language) {
            continue;
        }

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        for specifier in extract_import_specifiers(&content, language) {
            let Some(target_file) = resolve_import_path(&file.path, &specifier, language, &context)
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

pub(super) fn parse_calls(
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

        if !parser_loader::supports_call_heritage_extraction(language) {
            continue;
        }

        let should_extract_js_routes = matches!(
            language,
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript
        ) && looks_like_web_route_file(&file.path);

        let file_symbols = symbols_by_file.get(&file.path);

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

            if should_extract_js_routes {
                for handler_name in extract_js_route_handler_names(line) {
                    let Some((target_symbol_id, confidence)) = resolve_callee_symbol_id(
                        &file.path,
                        &handler_name,
                        &symbols_by_file_name,
                        &imports_by_source,
                    ) else {
                        continue;
                    };

                    let source_id = format!("File:{}", file.path);
                    let key = format!("{source_id}->{target_symbol_id}");
                    if !seen.insert(key) {
                        continue;
                    }

                    calls.push(ParsedCall {
                        source_symbol_id: source_id,
                        target_symbol_id,
                        confidence,
                    });
                }
            }

            let Some(caller_symbol) =
                file_symbols.and_then(|symbols| find_caller_symbol(symbols, line_no))
            else {
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

    for file in files {
        let Some(language) = file.language else {
            continue;
        };
        if language != SupportedLanguage::PHP || !looks_like_laravel_route_file(&file.path) {
            continue;
        }

        let full_path = repo_path.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        for handler in extract_laravel_route_handlers(&content) {
            let Some((target_symbol_id, confidence)) = resolve_laravel_route_method_id(
                &file.path,
                &handler,
                &symbols_by_file_name,
                &imports_by_source,
            ) else {
                continue;
            };

            let source_id = format!("File:{}", file.path);
            let key = format!("{source_id}->{target_symbol_id}");
            if !seen.insert(key) {
                continue;
            }

            calls.push(ParsedCall {
                source_symbol_id: source_id,
                target_symbol_id,
                confidence,
            });
        }
    }

    calls.sort_by(|a, b| {
        a.source_symbol_id
            .cmp(&b.source_symbol_id)
            .then(a.target_symbol_id.cmp(&b.target_symbol_id))
    });
    Ok(calls)
}

pub(super) fn parse_heritage(
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

        if !parser_loader::supports_call_heritage_extraction(language) {
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

            if let Some(source_name) = super::extract_identifier_after_keyword(line, "class ") {
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

            if let Some(source_name) = super::extract_identifier_after_keyword(line, "interface ") {
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
        .collect::<Vec<_>>()
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

fn looks_like_web_route_file(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/").to_ascii_lowercase();
    let is_js_like = normalized.ends_with(".ts")
        || normalized.ends_with(".tsx")
        || normalized.ends_with(".js")
        || normalized.ends_with(".jsx")
        || normalized.ends_with(".mjs")
        || normalized.ends_with(".cjs");
    if !is_js_like {
        return false;
    }

    let file_name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

    normalized.starts_with("routes/")
        || normalized.contains("/routes/")
        || normalized.starts_with("router/")
        || normalized.contains("/router/")
        || file_name.contains("route")
        || file_name.contains("router")
}

fn extract_js_route_handler_names(line: &str) -> Vec<String> {
    const ROUTE_CALL_PATTERNS: &[&str] = &[
        ".get(",
        ".post(",
        ".put(",
        ".patch(",
        ".delete(",
        ".options(",
        ".head(",
        ".all(",
        ".use(",
    ];

    let mut handlers = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();

    for pattern in ROUTE_CALL_PATTERNS {
        let mut search_start = 0usize;
        while search_start < line.len() {
            let Some(relative) = line.get(search_start..).and_then(|s| s.find(pattern)) else {
                break;
            };
            let open_index = search_start + relative + pattern.len() - 1;
            let Some(close_index) = find_matching_delimiter(line, open_index, b'(', b')') else {
                break;
            };
            let Some(args_body) = line.get(open_index + 1..close_index) else {
                search_start = close_index.saturating_add(1);
                continue;
            };

            let args = split_top_level_segments(args_body, ',');
            let first_handler_index = if pattern == &".use(" && args.len() == 1 {
                0
            } else {
                1
            };
            for arg in args.into_iter().skip(first_handler_index) {
                for handler in extract_js_handler_from_expr(&arg) {
                    if seen.insert(handler.clone()) {
                        handlers.push(handler);
                    }
                }
            }

            search_start = close_index.saturating_add(1);
        }
    }

    handlers
}

fn extract_js_handler_from_expr(expr: &str) -> Vec<String> {
    let raw = trim_wrapping_parentheses(expr.trim());
    if raw.is_empty() {
        return Vec::new();
    }
    if raw.starts_with('\'') || raw.starts_with('"') || raw.starts_with('`') {
        return Vec::new();
    }
    if raw.contains("=>")
        || raw.starts_with("function ")
        || raw.starts_with("async function ")
        || raw.starts_with("async (")
    {
        return Vec::new();
    }

    if raw.starts_with('[') && raw.ends_with(']') {
        let mut out = Vec::<String>::new();
        let mut seen = HashSet::<String>::new();
        if let Some(inner) = raw.get(1..raw.len().saturating_sub(1)) {
            for part in split_top_level_segments(inner, ',') {
                for name in extract_js_handler_from_expr(&part) {
                    if seen.insert(name.clone()) {
                        out.push(name);
                    }
                }
            }
        }
        return out;
    }

    if let Some(open_index) = raw.find('(') {
        if let Some(close_index) = find_matching_delimiter(raw, open_index, b'(', b')') {
            if close_index + 1 == raw.len() {
                let mut out = Vec::<String>::new();
                let mut seen = HashSet::<String>::new();
                if let Some(inner) = raw.get(open_index + 1..close_index) {
                    for part in split_top_level_segments(inner, ',') {
                        for name in extract_js_handler_from_expr(&part) {
                            if seen.insert(name.clone()) {
                                out.push(name);
                            }
                        }
                    }
                }
                if !out.is_empty() {
                    return out;
                }
            }
        }
    }

    let mut token = raw;
    token = token.trim_start_matches("await ").trim_start();
    token = token.trim_start_matches("return ").trim_start();
    token = token.trim_start_matches("void ").trim_start();
    token = token.trim_end_matches(|ch: char| matches!(ch, ';' | ',' | ')'));
    if let Some((before, _)) = token.split_once(".bind(") {
        token = before.trim();
    }

    let normalized = token.replace("?.", ".");
    let candidate = normalized
        .rsplit('.')
        .next()
        .unwrap_or(normalized.as_str())
        .split('[')
        .next()
        .unwrap_or("")
        .trim();
    if candidate.is_empty()
        || matches!(
            candidate,
            "req" | "res" | "next" | "ctx" | "request" | "response"
        )
    {
        return Vec::new();
    }

    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return Vec::new();
    };
    if !is_identifier_start(first) {
        return Vec::new();
    }

    let mut ident = String::new();
    ident.push(first);
    for ch in chars {
        if !is_identifier_continue(ch) {
            break;
        }
        ident.push(ch);
    }

    if ident.is_empty() || is_non_call_keyword(&ident) {
        Vec::new()
    } else {
        vec![ident]
    }
}

fn trim_wrapping_parentheses(input: &str) -> &str {
    let mut current = input.trim();
    loop {
        if !(current.starts_with('(') && current.ends_with(')')) {
            return current;
        }
        let Some(close_index) = find_matching_delimiter(current, 0, b'(', b')') else {
            return current;
        };
        if close_index + 1 != current.len() {
            return current;
        }
        current = current
            .get(1..current.len().saturating_sub(1))
            .unwrap_or_default()
            .trim();
    }
}

fn split_top_level_segments(input: &str, separator: char) -> Vec<String> {
    let bytes = input.as_bytes();
    let separator = separator as u8;
    let mut out = Vec::<String>::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quoted: Option<u8> = None;
    let mut escaped = false;

    for (idx, byte) in bytes.iter().copied().enumerate() {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == quote {
                quoted = None;
            }
            continue;
        }

        match byte {
            b'\'' | b'"' | b'`' => quoted = Some(byte),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }

        if byte == separator
            && paren_depth == 0
            && bracket_depth == 0
            && brace_depth == 0
            && quoted.is_none()
        {
            let part = input.get(start..idx).unwrap_or_default().trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
            start = idx + 1;
        }
    }

    let tail = input.get(start..).unwrap_or_default().trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }

    out
}

fn find_matching_delimiter(text: &str, open_index: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open_index).copied()? != open {
        return None;
    }

    let mut depth = 0usize;
    let mut quoted: Option<u8> = None;
    let mut escaped = false;

    for (idx, byte) in bytes.iter().copied().enumerate().skip(open_index) {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == quote {
                quoted = None;
            }
            continue;
        }

        match byte {
            b'\'' | b'"' | b'`' => quoted = Some(byte),
            _ if byte == open => depth += 1,
            _ if byte == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }

    None
}

#[derive(Debug, Clone)]
struct LaravelRouteHandler {
    controller_name: String,
    method_name: String,
}

fn looks_like_laravel_route_file(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/").to_ascii_lowercase();
    normalized.ends_with(".php")
        && (normalized.starts_with("routes/") || normalized.contains("/routes/"))
}

fn extract_laravel_route_handlers(content: &str) -> Vec<LaravelRouteHandler> {
    const RESOURCE_METHODS: &[&str] = &[
        "index", "create", "store", "show", "edit", "update", "destroy",
    ];
    const API_RESOURCE_METHODS: &[&str] = &["index", "store", "show", "update", "destroy"];

    let mut handlers = Vec::<LaravelRouteHandler>::new();
    for raw_line in content.lines() {
        let line = strip_inline_comment(raw_line).trim();
        if line.is_empty() || !line.contains("Route::") || !line.contains("::class") {
            continue;
        }

        let Some(controller_name) = extract_laravel_controller_name(line) else {
            continue;
        };

        if line.contains("Route::resource(") {
            for method in RESOURCE_METHODS {
                handlers.push(LaravelRouteHandler {
                    controller_name: controller_name.clone(),
                    method_name: (*method).to_string(),
                });
            }
            continue;
        }

        if line.contains("Route::apiResource(") {
            for method in API_RESOURCE_METHODS {
                handlers.push(LaravelRouteHandler {
                    controller_name: controller_name.clone(),
                    method_name: (*method).to_string(),
                });
            }
            continue;
        }

        if let Some(method_name) = extract_laravel_controller_method_name(line) {
            handlers.push(LaravelRouteHandler {
                controller_name,
                method_name,
            });
        }
    }

    handlers
}

fn extract_laravel_controller_name(line: &str) -> Option<String> {
    let class_index = line.find("::class")?;
    let before = line.get(..class_index)?.trim_end();
    let split_at = before.rfind('[').or_else(|| before.rfind(','));
    let token = match split_at {
        Some(idx) => before.get(idx + 1..).unwrap_or(before).trim(),
        None => before.trim(),
    };
    normalize_php_class_name(token)
}

fn extract_laravel_controller_method_name(line: &str) -> Option<String> {
    let class_index = line.find("::class")?;
    let rest = line.get(class_index + "::class".len()..)?;
    extract_quoted_literal(rest)
}

fn normalize_php_class_name(raw: &str) -> Option<String> {
    let mut token = raw.trim().trim_start_matches('\\');
    if let Some((head, _)) = token.split_once("::") {
        token = head.trim();
    }
    token = token.trim_end_matches(|ch| matches!(ch, ',' | ')' | ']'));

    let class_name = token.rsplit('\\').next().unwrap_or_default().trim();
    if class_name.is_empty() {
        return None;
    }

    let mut normalized = String::new();
    for ch in class_name.chars() {
        if is_identifier_continue(ch) {
            normalized.push(ch);
        } else {
            break;
        }
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn resolve_laravel_route_method_id(
    source_file: &str,
    handler: &LaravelRouteHandler,
    symbols_by_file_name: &BTreeMap<String, BTreeMap<String, Vec<&ParsedSymbol>>>,
    imports_by_source: &BTreeMap<String, Vec<String>>,
) -> Option<(String, f32)> {
    let mut candidate_files = imports_by_source
        .get(source_file)
        .cloned()
        .unwrap_or_default();
    if !candidate_files.iter().any(|path| path == source_file) {
        candidate_files.push(source_file.to_string());
    }

    for file_path in &candidate_files {
        let has_controller = resolve_symbol_id_in_file(
            file_path,
            &handler.controller_name,
            Some("Class"),
            symbols_by_file_name,
        )
        .is_some();
        if !has_controller {
            continue;
        }

        if let Some(method_id) = resolve_symbol_id_in_file(
            file_path,
            &handler.method_name,
            Some("Function"),
            symbols_by_file_name,
        )
        .or_else(|| {
            resolve_symbol_id_in_file(file_path, &handler.method_name, None, symbols_by_file_name)
        }) {
            return Some((method_id, 0.75));
        }
    }

    for file_path in symbols_by_file_name.keys() {
        let has_controller = resolve_symbol_id_in_file(
            file_path,
            &handler.controller_name,
            Some("Class"),
            symbols_by_file_name,
        )
        .is_some();
        if !has_controller {
            continue;
        }

        if let Some(method_id) = resolve_symbol_id_in_file(
            file_path,
            &handler.method_name,
            Some("Function"),
            symbols_by_file_name,
        )
        .or_else(|| {
            resolve_symbol_id_in_file(file_path, &handler.method_name, None, symbols_by_file_name)
        }) {
            return Some((method_id, 0.65));
        }
    }

    None
}

fn extract_import_specifiers(content: &str, language: SupportedLanguage) -> Vec<String> {
    let mut specifiers = Vec::<String>::new();
    let mut in_go_import_block = false;

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
            SupportedLanguage::Go => {
                if in_go_import_block {
                    if line.starts_with(')') {
                        in_go_import_block = false;
                        continue;
                    }
                    if let Some(spec) = extract_quoted_literal(line) {
                        specifiers.push(spec);
                    }
                    continue;
                }

                if !line.starts_with("import") {
                    continue;
                }

                let rest = line.trim_start_matches("import").trim_start();
                if rest.starts_with('(') {
                    in_go_import_block = true;
                    continue;
                }

                if let Some(spec) = extract_quoted_literal(rest) {
                    specifiers.push(spec);
                }
            }
            SupportedLanguage::PHP => {
                specifiers.extend(extract_php_use_imports(line));
            }
            SupportedLanguage::CSharp => {
                if let Some(spec) = extract_csharp_using_import(line) {
                    specifiers.push(spec);
                }
            }
            _ => {}
        }
    }

    specifiers
}

fn extract_php_use_imports(line: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    if !line.starts_with("use ") {
        return out;
    }

    let mut body = line
        .trim_start_matches("use ")
        .trim_end_matches(';')
        .trim()
        .to_string();
    if body.is_empty() || body.starts_with('(') || body.starts_with('$') {
        return out;
    }

    if let Some(rest) = body.strip_prefix("function ") {
        body = rest.trim().to_string();
    } else if let Some(rest) = body.strip_prefix("const ") {
        body = rest.trim().to_string();
    }

    if let Some((prefix, members)) = body.split_once('{') {
        if let Some((inner, _)) = members.split_once('}') {
            let namespace = prefix.trim().trim_end_matches('\\');
            for member in inner.split(',') {
                let member = strip_php_alias(member);
                if member.is_empty() {
                    continue;
                }
                out.push(format!("{namespace}\\{member}"));
            }
            return out;
        }
    }

    for part in body.split(',') {
        let namespace = strip_php_alias(part);
        if namespace.is_empty() {
            continue;
        }
        out.push(namespace.to_string());
    }

    out
}

fn strip_php_alias(segment: &str) -> &str {
    let trimmed = segment.trim().trim_start_matches('\\');
    if trimmed.is_empty() {
        return "";
    }

    if let Some(index) = find_case_insensitive(trimmed, " as ") {
        return trimmed.get(..index).unwrap_or("").trim();
    }

    trimmed
}

fn extract_csharp_using_import(line: &str) -> Option<String> {
    if !line.starts_with("using ") {
        return None;
    }

    let mut body = line
        .trim_start_matches("using ")
        .trim_end_matches(';')
        .trim()
        .to_string();
    if body.is_empty() || body.starts_with('(') {
        return None;
    }

    if let Some(rest) = body.strip_prefix("static ") {
        body = rest.trim().to_string();
    }

    if let Some((_, rhs)) = body.split_once('=') {
        body = rhs.trim().to_string();
    }

    if body.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '<' || ch == '>'
    }) {
        return Some(body);
    }

    None
}

fn find_case_insensitive(text: &str, needle: &str) -> Option<usize> {
    let lowered = text.to_ascii_lowercase();
    lowered.find(&needle.to_ascii_lowercase())
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

fn resolve_import_path(
    source_file: &str,
    specifier: &str,
    language: SupportedLanguage,
    context: &ImportResolutionContext,
) -> Option<String> {
    let specifier = specifier.trim();
    if specifier.is_empty() {
        return None;
    }

    match language {
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
            if specifier.starts_with('.') {
                return resolve_relative_import_path(
                    source_file,
                    specifier,
                    language,
                    &context.file_set,
                );
            }

            if let Some(tsconfig) = &context.tsconfig_paths {
                if let Some(resolved) =
                    resolve_tsconfig_alias_import(specifier, language, context, tsconfig)
                {
                    return Some(resolved);
                }

                if let Some(resolved) =
                    resolve_base_url_import(specifier, language, context, tsconfig)
                {
                    return Some(resolved);
                }
            }

            resolve_with_suffix_index(specifier, language, context)
        }
        SupportedLanguage::Go => resolve_go_import_path(specifier, context),
        SupportedLanguage::PHP => resolve_php_import_path(specifier, context),
        SupportedLanguage::CSharp => resolve_csharp_import_path(source_file, specifier, context),
        _ => None,
    }
}

fn resolve_go_import_path(specifier: &str, context: &ImportResolutionContext) -> Option<String> {
    let module_path = context.go_module_path.as_deref()?;
    let remainder = match specifier.strip_prefix(module_path) {
        Some("") => "",
        Some(rest) => rest.trim_start_matches('/'),
        None => return None,
    };
    if remainder.is_empty() {
        return None;
    }

    let normalized = normalize_relative_path(PathBuf::from(remainder));
    if normalized.is_empty() {
        return None;
    }

    resolve_with_base_candidate(&normalized, SupportedLanguage::Go, context)
}

fn resolve_php_import_path(specifier: &str, context: &ImportResolutionContext) -> Option<String> {
    let normalized = specifier.trim().trim_start_matches('\\');
    if normalized.is_empty() {
        return None;
    }

    for mapping in &context.composer_psr4 {
        let Some(remainder) = match_namespace_remainder(normalized, &mapping.namespace_prefix)
        else {
            continue;
        };

        let relative = remainder.replace('\\', "/");
        let candidate = join_path_segments(&mapping.directory_prefix, &relative);
        if let Some(resolved) =
            resolve_with_base_candidate(&candidate, SupportedLanguage::PHP, context)
        {
            return Some(resolved);
        }
    }

    let fallback = normalized.replace('\\', "/");
    resolve_with_suffix_index(&fallback, SupportedLanguage::PHP, context)
}

fn resolve_csharp_import_path(
    source_file: &str,
    specifier: &str,
    context: &ImportResolutionContext,
) -> Option<String> {
    if let Some(project) = find_csharp_project(source_file, &context.csharp_projects) {
        if let Some(resolved) = resolve_csharp_namespace_in_project(specifier, project, context) {
            return Some(resolved);
        }
    }

    for project in &context.csharp_projects {
        if let Some(resolved) = resolve_csharp_namespace_in_project(specifier, project, context) {
            return Some(resolved);
        }
    }

    let fallback = specifier.replace('.', "/");
    resolve_with_suffix_index(&fallback, SupportedLanguage::CSharp, context)
}

fn find_csharp_project<'a>(
    source_file: &str,
    projects: &'a [CSharpProjectConfig],
) -> Option<&'a CSharpProjectConfig> {
    projects
        .iter()
        .filter(|project| is_path_within_project(source_file, &project.project_dir))
        .max_by_key(|project| project.project_dir.len())
}

fn is_path_within_project(file_path: &str, project_dir: &str) -> bool {
    if project_dir.is_empty() {
        return true;
    }
    file_path == project_dir || file_path.starts_with(&format!("{project_dir}/"))
}

fn resolve_csharp_namespace_in_project(
    specifier: &str,
    project: &CSharpProjectConfig,
    context: &ImportResolutionContext,
) -> Option<String> {
    let remainder = match_dot_namespace_remainder(specifier, &project.root_namespace)?;
    if remainder.is_empty() {
        return None;
    }

    let dotted = remainder.replace('.', "/");
    let candidate = if project.project_dir.is_empty() {
        dotted
    } else {
        join_path_segments(&project.project_dir, &dotted)
    };

    resolve_with_base_candidate(&candidate, SupportedLanguage::CSharp, context)
}

fn resolve_tsconfig_alias_import(
    specifier: &str,
    language: SupportedLanguage,
    context: &ImportResolutionContext,
    tsconfig: &TsconfigPaths,
) -> Option<String> {
    for (alias_prefix, target_prefix) in &tsconfig.aliases {
        let Some(remainder) = match_alias_remainder(specifier, alias_prefix) else {
            continue;
        };

        let mapped = join_path_segments(target_prefix, remainder);
        let rewritten = if tsconfig.base_url == "." {
            mapped
        } else {
            join_path_segments(&tsconfig.base_url, &mapped)
        };

        if let Some(resolved) = resolve_with_base_candidate(&rewritten, language, context) {
            return Some(resolved);
        }
    }

    None
}

fn resolve_base_url_import(
    specifier: &str,
    language: SupportedLanguage,
    context: &ImportResolutionContext,
    tsconfig: &TsconfigPaths,
) -> Option<String> {
    let rewritten = if tsconfig.base_url == "." {
        specifier.to_string()
    } else {
        join_path_segments(&tsconfig.base_url, specifier)
    };

    resolve_with_base_candidate(&rewritten, language, context)
}

fn resolve_with_base_candidate(
    candidate: &str,
    language: SupportedLanguage,
    context: &ImportResolutionContext,
) -> Option<String> {
    let normalized = normalize_relative_path(PathBuf::from(candidate));
    if normalized.is_empty() {
        return None;
    }

    if let Some(resolved) = resolve_import_base_path(&normalized, language, &context.file_set) {
        return Some(resolved);
    }

    resolve_suffix_for_base(&normalized, language, &context.suffix_index)
}

fn resolve_with_suffix_index(
    specifier: &str,
    language: SupportedLanguage,
    context: &ImportResolutionContext,
) -> Option<String> {
    let path_like = if specifier.contains('/') {
        specifier.to_string()
    } else {
        specifier.replace('.', "/")
    };
    let normalized = normalize_relative_path(PathBuf::from(path_like));
    if normalized.is_empty() {
        return None;
    }

    resolve_suffix_for_base(&normalized, language, &context.suffix_index)
}

fn resolve_suffix_for_base(
    base: &str,
    language: SupportedLanguage,
    suffix_index: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(path) = suffix_index.get(base) {
        return Some(path.clone());
    }

    for ext in extension_candidates(language) {
        let candidate = format!("{base}{ext}");
        if let Some(path) = suffix_index.get(&candidate) {
            return Some(path.clone());
        }
    }

    for ext in extension_candidates(language) {
        let candidate = format!("{base}/index{ext}");
        if let Some(path) = suffix_index.get(&candidate) {
            return Some(path.clone());
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

    resolve_import_base_path(&import_base, language, file_set)
}

fn resolve_import_base_path(
    import_base: &str,
    language: SupportedLanguage,
    file_set: &HashSet<String>,
) -> Option<String> {
    if file_set.contains(import_base) {
        return Some(import_base.to_string());
    }

    for ext in extension_candidates(language) {
        let candidate = format!("{import_base}{ext}");
        if file_set.contains(&candidate) {
            return Some(candidate);
        }
    }

    if let Some(candidate) = resolve_directory_module_file(import_base, language, file_set) {
        return Some(candidate);
    }

    for ext in extension_candidates(language) {
        let candidate = format!("{import_base}/index{ext}");
        if file_set.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn extension_candidates(language: SupportedLanguage) -> &'static [&'static str] {
    match language {
        SupportedLanguage::TypeScript => {
            &[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"]
        }
        SupportedLanguage::JavaScript => {
            &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"]
        }
        SupportedLanguage::Go => &[".go"],
        SupportedLanguage::PHP => &[".php"],
        SupportedLanguage::CSharp => &[".cs"],
        _ => &[],
    }
}

fn resolve_directory_module_file(
    import_base: &str,
    language: SupportedLanguage,
    file_set: &HashSet<String>,
) -> Option<String> {
    if !matches!(
        language,
        SupportedLanguage::Go | SupportedLanguage::CSharp | SupportedLanguage::PHP
    ) {
        return None;
    }

    let base = import_base.trim_matches('/');
    if base.is_empty() {
        return None;
    }

    let prefix = format!("{base}/");
    let mut candidates = file_set
        .iter()
        .filter(|path| {
            path.starts_with(&prefix)
                && extension_candidates(language)
                    .iter()
                    .any(|ext| path.ends_with(ext))
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    candidates.sort();

    let leaf = base.rsplit('/').next().unwrap_or_default();
    let preferred = match language {
        SupportedLanguage::Go => format!("{prefix}{leaf}.go"),
        SupportedLanguage::CSharp => format!("{prefix}{leaf}.cs"),
        SupportedLanguage::PHP => format!("{prefix}{leaf}.php"),
        _ => String::new(),
    };
    if !preferred.is_empty() && file_set.contains(&preferred) {
        return Some(preferred);
    }

    if matches!(language, SupportedLanguage::Go) {
        if let Some(non_test) = candidates.iter().find(|path| !path.ends_with("_test.go")) {
            return Some(non_test.clone());
        }
    }

    candidates.into_iter().next()
}

fn match_namespace_remainder<'a>(specifier: &'a str, namespace_prefix: &str) -> Option<&'a str> {
    let spec = specifier.trim_start_matches('\\');
    let prefix = namespace_prefix
        .trim_start_matches('\\')
        .trim_end_matches('\\');
    if prefix.is_empty() {
        return Some(spec);
    }
    if spec == prefix {
        return Some("");
    }
    let remainder = spec.strip_prefix(prefix)?;
    remainder.strip_prefix('\\')
}

fn match_dot_namespace_remainder<'a>(
    specifier: &'a str,
    namespace_prefix: &str,
) -> Option<&'a str> {
    let spec = specifier.trim();
    let prefix = namespace_prefix.trim().trim_end_matches('.');
    if prefix.is_empty() {
        return Some(spec);
    }
    if spec == prefix {
        return Some("");
    }
    let remainder = spec.strip_prefix(prefix)?;
    remainder.strip_prefix('.')
}

fn match_alias_remainder<'a>(specifier: &'a str, alias_prefix: &str) -> Option<&'a str> {
    if alias_prefix.ends_with('/') {
        return specifier.strip_prefix(alias_prefix);
    }
    if specifier == alias_prefix {
        return Some("");
    }
    let remainder = specifier.strip_prefix(alias_prefix)?;
    remainder.strip_prefix('/')
}

fn join_path_segments(prefix: &str, remainder: &str) -> String {
    let left = prefix.trim_matches('/');
    let right = remainder.trim_matches('/');

    match (left.is_empty(), right.is_empty()) {
        (true, true) => String::new(),
        (true, false) => right.to_string(),
        (false, true) => left.to_string(),
        (false, false) => format!("{left}/{right}"),
    }
}

fn load_tsconfig_paths(repo_root: &Path) -> Option<TsconfigPaths> {
    const CANDIDATES: &[&str] = &["tsconfig.json", "tsconfig.app.json", "tsconfig.base.json"];
    let mut base_only: Option<TsconfigPaths> = None;

    for filename in CANDIDATES {
        let path = repo_root.join(filename);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        let stripped = strip_json_comments(&raw);
        let parsed = match serde_json::from_str::<TsconfigFile>(&stripped) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        let options = match parsed.compiler_options {
            Some(options) => options,
            None => continue,
        };

        let base_url = normalize_base_url(options.base_url.as_deref().unwrap_or("."));
        let mut aliases = Vec::<(String, String)>::new();
        if let Some(paths) = options.paths {
            for (pattern, targets) in paths {
                let Some(first_target) = targets.first() else {
                    continue;
                };

                let alias_prefix = normalize_alias_pattern(&pattern);
                let target_prefix = normalize_target_pattern(first_target);
                if alias_prefix.is_empty() || target_prefix.is_empty() {
                    continue;
                }
                aliases.push((alias_prefix, target_prefix));
            }
        }

        aliases.sort_by(|a, b| {
            b.0.len()
                .cmp(&a.0.len())
                .then(a.0.cmp(&b.0))
                .then(a.1.cmp(&b.1))
        });

        if !aliases.is_empty() {
            return Some(TsconfigPaths { aliases, base_url });
        }

        if options.base_url.is_some() && base_only.is_none() {
            base_only = Some(TsconfigPaths {
                aliases: Vec::new(),
                base_url,
            });
        }
    }

    base_only
}

fn load_go_module_path(repo_root: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(repo_root.join("go.mod")).ok()?;
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(module) = line.strip_prefix("module ") {
            let module = module.trim();
            if !module.is_empty() {
                return Some(module.to_string());
            }
        }
    }

    None
}

fn load_composer_psr4(repo_root: &Path) -> Vec<ComposerPsr4Mapping> {
    let raw = match std::fs::read_to_string(repo_root.join("composer.json")) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };

    let parsed = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(parsed) => parsed,
        Err(_) => return Vec::new(),
    };

    let psr4 = match parsed
        .get("autoload")
        .and_then(|autoload| autoload.get("psr-4"))
        .and_then(|value| value.as_object())
    {
        Some(psr4) => psr4,
        None => return Vec::new(),
    };

    let mut mappings = Vec::<ComposerPsr4Mapping>::new();
    for (namespace_prefix, path_value) in psr4 {
        let namespace_prefix = namespace_prefix
            .trim()
            .trim_start_matches('\\')
            .trim_end_matches('\\')
            .to_string();
        if namespace_prefix.is_empty() {
            continue;
        }

        let raw_dir = if let Some(path) = path_value.as_str() {
            path.to_string()
        } else if let Some(paths) = path_value.as_array() {
            paths
                .first()
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        };
        if raw_dir.is_empty() {
            continue;
        }

        let normalized_dir = normalize_relative_path(PathBuf::from(raw_dir.trim()));
        mappings.push(ComposerPsr4Mapping {
            namespace_prefix,
            directory_prefix: normalized_dir,
        });
    }

    mappings.sort_by(|a, b| {
        b.namespace_prefix
            .len()
            .cmp(&a.namespace_prefix.len())
            .then(a.namespace_prefix.cmp(&b.namespace_prefix))
            .then(a.directory_prefix.cmp(&b.directory_prefix))
    });
    mappings
}

fn load_csharp_projects(repo_root: &Path) -> Vec<CSharpProjectConfig> {
    let mut projects = Vec::<CSharpProjectConfig>::new();

    for entry in WalkDir::new(repo_root)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("csproj") {
            continue;
        }

        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        let project_dir = path
            .parent()
            .and_then(|parent| parent.strip_prefix(repo_root).ok())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        let root_namespace = extract_xml_tag_value(&raw, "RootNamespace").or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
        });
        let Some(root_namespace) = root_namespace else {
            continue;
        };

        let root_namespace = root_namespace.trim().to_string();
        if root_namespace.is_empty() {
            continue;
        }

        projects.push(CSharpProjectConfig {
            project_dir: normalize_relative_path(PathBuf::from(project_dir)),
            root_namespace,
        });
    }

    projects.sort_by(|a, b| {
        b.project_dir
            .len()
            .cmp(&a.project_dir.len())
            .then(a.project_dir.cmp(&b.project_dir))
            .then(a.root_namespace.cmp(&b.root_namespace))
    });
    projects
        .dedup_by(|a, b| a.project_dir == b.project_dir && a.root_namespace == b.root_namespace);
    projects
}

fn extract_xml_tag_value(content: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}");
    let start = content.find(&start_tag)?;
    let after_start = content.get(start + start_tag.len()..)?;
    let close_angle = after_start.find('>')?;
    let body = after_start.get(close_angle + 1..)?;

    let end_tag = format!("</{tag}>");
    let end = body.find(&end_tag)?;
    let value = body.get(..end)?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn strip_json_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars = raw.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                out.push('\n');
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if ch == '*' && next == Some('/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '/' && next == Some('/') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if ch == '/' && next == Some('*') {
            in_block_comment = true;
            i += 2;
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        return ".".to_string();
    }

    let normalized = normalize_relative_path(PathBuf::from(trimmed));
    if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized
    }
}

fn normalize_alias_pattern(pattern: &str) -> String {
    let trimmed = pattern.trim().replace('\\', "/");
    if let Some(prefix) = trimmed.strip_suffix("/*") {
        return format!("{}/", prefix.trim_end_matches('/'));
    }
    trimmed
}

fn normalize_target_pattern(target: &str) -> String {
    let trimmed = target.trim().replace('\\', "/");
    let trimmed = trimmed.trim_start_matches("./").trim_start_matches('/');
    if let Some(prefix) = trimmed.strip_suffix("/*") {
        return format!("{}/", prefix.trim_end_matches('/'));
    }
    trimmed.to_string()
}

fn build_suffix_index(file_list: &[String]) -> BTreeMap<String, String> {
    let mut index = BTreeMap::<String, String>::new();

    for file in file_list {
        let normalized = file.replace('\\', "/");
        let parts = normalized
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }

        for start in 0..parts.len() {
            let suffix = parts[start..].join("/");
            index.entry(suffix).or_insert_with(|| file.clone());
        }
    }

    index
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
