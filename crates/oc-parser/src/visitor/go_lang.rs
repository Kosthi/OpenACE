use std::path::PathBuf;

use oc_core::{CodeRelation, CodeSymbol, Language, QualifiedName, RelationKind, SymbolId, SymbolKind};

use crate::error::ParserError;
use crate::visitor::{ParseOutput, VisitorContext};

pub(crate) fn extract(
    ctx: &VisitorContext<'_>,
    tree: &tree_sitter::Tree,
) -> Result<ParseOutput, ParserError> {
    let mut symbols = Vec::new();
    let mut relations = Vec::new();

    let root = tree.root_node();
    let package_name = extract_package_name(ctx, root);

    let pkg_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &package_name,
        root.start_byte(),
        root.end_byte(),
    );
    symbols.push(CodeSymbol {
        id: pkg_id,
        name: package_name.clone(),
        qualified_name: package_name.clone(),
        kind: SymbolKind::Package,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: root.start_byte()..root.end_byte(),
        line_range: root.start_position().row as u32..root.end_position().row as u32 + 1,
        signature: None,
        doc_comment: None,
        body_text: None,
        body_hash: ctx.body_hash(root),
    });

    let scope = vec![package_name.as_str()];
    extract_declarations(ctx, root, &scope, pkg_id, &mut symbols, &mut relations);

    Ok(ParseOutput { symbols, relations })
}

fn extract_declarations(
    ctx: &VisitorContext<'_>,
    parent: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                extract_function(ctx, child, scope, scope_id, symbols, relations);
            }
            "method_declaration" => {
                extract_method(ctx, child, scope, scope_id, symbols, relations);
            }
            "type_declaration" => {
                extract_type_decl(ctx, child, scope, scope_id, symbols, relations);
            }
            "const_declaration" | "var_declaration" => {
                extract_var_or_const(ctx, child, scope, scope_id, symbols, relations);
            }
            "import_declaration" => {
                extract_import(ctx, child, scope_id, relations);
            }
            _ => {}
        }
    }
}

fn extract_function(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => ctx.node_text(n),
        None => return,
    };

    if name.is_empty() {
        return;
    }

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    let signature = build_func_signature(ctx, node, name);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Function,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: extract_go_doc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_go(ctx, body, sym_id, relations);
    }
}

fn extract_method(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => ctx.node_text(n),
        None => return,
    };

    if name.is_empty() {
        return;
    }

    // Get the receiver type to scope the method
    let receiver_type = node
        .child_by_field_name("receiver")
        .and_then(|r| extract_receiver_type(ctx, r))
        .unwrap_or_default();

    let method_scope = if receiver_type.is_empty() {
        scope.to_vec()
    } else {
        let mut s = scope.to_vec();
        s.push(&receiver_type);
        s
    };

    let qname = qualified_name(&method_scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    let signature = build_func_signature(ctx, node, name);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Method,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: extract_go_doc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_go(ctx, body, sym_id, relations);
    }
}

fn extract_receiver_type<'a>(ctx: &VisitorContext<'_>, receiver: tree_sitter::Node<'a>) -> Option<String> {
    // parameter_list containing (name *Type) or (name Type) or (name *Type[T])
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_text = ctx.node_text(type_node);
                // Strip pointer * prefix and generic type params
                let clean = type_text.trim_start_matches('*');
                let clean = clean.split('[').next().unwrap_or(clean).trim();
                return Some(clean.to_string());
            }
        }
    }
    None
}

fn extract_type_decl(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    // type_declaration contains type_spec children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            extract_type_spec(ctx, child, scope, scope_id, symbols, relations);
        }
    }
}

fn extract_type_spec(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => ctx.node_text(n),
        None => return,
    };

    let type_node = node.child_by_field_name("type");
    let type_kind_str = type_node.map(|n| n.kind()).unwrap_or("");

    let kind = match type_kind_str {
        "struct_type" => SymbolKind::Struct,
        "interface_type" => SymbolKind::Interface,
        _ => SymbolKind::TypeAlias,
    };

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    let sig = match kind {
        SymbolKind::Struct => format!("type {name} struct"),
        SymbolKind::Interface => format!("type {name} interface"),
        _ => format!("type {name}"),
    };

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(sig),
        doc_comment: extract_go_doc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);
}

fn extract_var_or_const(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let is_const = node.kind() == "const_declaration";

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_spec" || child.kind() == "var_spec" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = ctx.node_text(name_node);
                if name.is_empty() || name == "_" {
                    continue;
                }

                let kind = if is_const {
                    SymbolKind::Constant
                } else {
                    SymbolKind::Variable
                };

                let qname = qualified_name(scope, name);
                let sym_id = SymbolId::generate(
                    ctx.repo_id, ctx.file_path, &qname,
                    child.start_byte(), child.end_byte(),
                );

                symbols.push(CodeSymbol {
                    id: sym_id,
                    name: name.to_string(),
                    qualified_name: qname,
                    kind,
                    language: ctx.language,
                    file_path: PathBuf::from(ctx.file_path),
                    byte_range: child.start_byte()..child.end_byte(),
                    line_range: child.start_position().row as u32..child.end_position().row as u32 + 1,
                    signature: None,
                    doc_comment: None,
                    body_text: None,
        body_hash: ctx.body_hash(child),
                });

                push_contains(ctx, scope_id, sym_id, child, relations);
            }
        }
    }
}

fn extract_import(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec_list" {
            let mut inner = child.walk();
            for spec in child.children(&mut inner) {
                if spec.kind() == "import_spec" {
                    extract_import_spec(ctx, spec, scope_id, relations);
                }
            }
        } else if child.kind() == "import_spec" {
            extract_import_spec(ctx, child, scope_id, relations);
        }
    }
}

fn extract_import_spec(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    if let Some(path_node) = node.child_by_field_name("path") {
        let path_text = ctx.node_text(path_node);
        let import_path = path_text.trim_matches('"');
        if !import_path.is_empty() {
            let canonical = QualifiedName::normalize(import_path, Language::Go);
            let target_id = SymbolId::generate("", "", &canonical, 0, 0);
            relations.push(CodeRelation {
                source_id: scope_id,
                target_id,
                kind: RelationKind::Imports,
                file_path: PathBuf::from(ctx.file_path),
                line: node.start_position().row as u32,
                confidence: RelationKind::Imports.default_confidence(),
            });
        }
    }
}

fn extract_calls_go(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    caller_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = child.child_by_field_name("function") {
                let callee = ctx.node_text(func);
                if !callee.is_empty() {
                    let target_id = SymbolId::generate("", "", callee, 0, 0);
                    relations.push(CodeRelation {
                        source_id: caller_id,
                        target_id,
                        kind: RelationKind::Calls,
                        file_path: PathBuf::from(ctx.file_path),
                        line: child.start_position().row as u32,
                        confidence: RelationKind::Calls.default_confidence(),
                    });
                }
            }
        }
        if child.kind() != "func_literal" {
            extract_calls_go(ctx, child, caller_id, relations);
        }
    }
}

// --- Helpers ---

fn push_contains(
    ctx: &VisitorContext<'_>,
    source_id: SymbolId,
    target_id: SymbolId,
    node: tree_sitter::Node<'_>,
    relations: &mut Vec<CodeRelation>,
) {
    relations.push(CodeRelation {
        source_id,
        target_id,
        kind: RelationKind::Contains,
        file_path: PathBuf::from(ctx.file_path),
        line: node.start_position().row as u32,
        confidence: RelationKind::Contains.default_confidence(),
    });
}

fn qualified_name(scope: &[&str], name: &str) -> String {
    let mut parts: Vec<&str> = scope.to_vec();
    parts.push(name);
    QualifiedName::join(&parts)
}

fn extract_package_name(ctx: &VisitorContext<'_>, root: tree_sitter::Node<'_>) -> String {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            let mut inner = child.walk();
            for pkg_child in child.children(&mut inner) {
                if pkg_child.kind() == "package_identifier" {
                    return ctx.node_text(pkg_child).to_string();
                }
            }
        }
    }
    // Fallback to file name
    let p = std::path::Path::new(ctx.file_path);
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string()
}

fn build_func_signature(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let result = node
        .child_by_field_name("result")
        .map(|n| format!(" {}", ctx.node_text(n)))
        .unwrap_or_default();
    format!("func {name}{params}{result}")
}

fn extract_go_doc(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>) -> Option<String> {
    // Walk backward collecting consecutive comment lines (Go doc convention)
    let mut lines = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = ctx.node_text(sibling);
            if let Some(stripped) = text.strip_prefix("//") {
                lines.push(stripped.trim().to_string());
            } else if text.starts_with("/*") {
                // Block comment: strip /* ... */ and extract content
                let inner = text
                    .strip_prefix("/*")
                    .and_then(|s| s.strip_suffix("*/"))
                    .unwrap_or(text);
                let cleaned: Vec<&str> = inner
                    .lines()
                    .map(|line| line.trim().trim_start_matches('*').trim())
                    .filter(|line| !line.is_empty())
                    .collect();
                for line in cleaned.into_iter().rev() {
                    lines.push(line.to_string());
                }
            } else {
                break;
            }
        } else {
            break;
        }
        prev = sibling.prev_sibling();
    }

    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}
