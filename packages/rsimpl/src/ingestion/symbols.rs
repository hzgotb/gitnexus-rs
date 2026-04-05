use std::path::Path;

use anyhow::Result;

use super::{ParsedSymbol, ScannedFile, SupportedLanguage, parser_loader};

pub(super) fn parse_symbols(repo_path: &Path, files: &[ScannedFile]) -> Result<Vec<ParsedSymbol>> {
    let mut symbols = Vec::<ParsedSymbol>::new();

    for file in files {
        let Some(language) = file.language else {
            continue;
        };
        if !parser_loader::supports_symbol_extraction(language) {
            continue;
        }

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

pub(super) fn extract_symbols_from_content(
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

pub(super) fn extract_identifier_after_keyword(line: &str, keyword: &str) -> Option<String> {
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
