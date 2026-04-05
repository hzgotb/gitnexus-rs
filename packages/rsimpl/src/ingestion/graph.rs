use std::collections::HashSet;

use super::{
    ParsedCall, ParsedHeritage, ParsedImport, ParsedSymbol, ScannedFile, StructureGraph,
    StructureNode, StructureRelationship, generate_id,
};

pub(super) fn build_structure_graph(
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
