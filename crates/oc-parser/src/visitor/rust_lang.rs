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
    let module_name = module_name_from_path(ctx.file_path);

    let module_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &module_name,
        root.start_byte(),
        root.end_byte(),
    );
    symbols.push(CodeSymbol {
        id: module_id,
        name: module_name.clone(),
        qualified_name: module_name.clone(),
        kind: SymbolKind::Module,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: root.start_byte()..root.end_byte(),
        line_range: root.start_position().row as u32..root.end_position().row as u32 + 1,
        signature: None,
        doc_comment: None,
        body_hash: ctx.body_hash(root),
    });

    let scope = vec![module_name.as_str()];
    extract_items(ctx, root, &scope, module_id, &mut symbols, &mut relations);

    Ok(ParseOutput { symbols, relations })
}

fn extract_items(
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
            "mod_item" => {
                extract_mod(ctx, child, scope, scope_id, symbols, relations);
            }
            "struct_item" => {
                extract_struct(ctx, child, scope, scope_id, symbols, relations);
            }
            "enum_item" => {
                extract_enum(ctx, child, scope, scope_id, symbols, relations);
            }
            "trait_item" => {
                extract_trait(ctx, child, scope, scope_id, symbols, relations);
            }
            "function_item" => {
                extract_function(ctx, child, scope, scope_id, false, symbols, relations);
            }
            "impl_item" => {
                extract_impl(ctx, child, scope, scope_id, symbols, relations);
            }
            "const_item" => {
                extract_const(ctx, child, scope, scope_id, symbols, relations);
            }
            "static_item" => {
                extract_const(ctx, child, scope, scope_id, symbols, relations);
            }
            "type_item" => {
                extract_type_alias(ctx, child, scope, scope_id, symbols, relations);
            }
            "use_declaration" => {
                extract_use(ctx, child, scope_id, relations);
            }
            _ => {}
        }
    }
}

fn extract_mod(
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

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Module,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("mod {name}")),
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    if let Some(body) = node.child_by_field_name("body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_items(ctx, body, &new_scope, sym_id, symbols, relations);
    }
}

fn extract_struct(
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

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Struct,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("struct {name}")),
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);
}

fn extract_enum(
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

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Enum,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);
}

fn extract_trait(
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

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Trait,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("trait {name}")),
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    // Extract methods within the trait body
    if let Some(body) = node.child_by_field_name("body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_trait_methods(ctx, body, &new_scope, sym_id, symbols, relations);
    }
}

fn extract_trait_methods(
    ctx: &VisitorContext<'_>,
    body: tree_sitter::Node<'_>,
    scope: &[&str],
    trait_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" || child.kind() == "function_signature_item" {
            extract_function(ctx, child, scope, trait_id, true, symbols, relations);
        } else if child.kind() == "type_item" {
            extract_type_alias(ctx, child, scope, trait_id, symbols, relations);
        } else if child.kind() == "const_item" {
            extract_const(ctx, child, scope, trait_id, symbols, relations);
        }
    }
}

fn extract_function(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    is_method: bool,
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

    let kind = if is_method {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    let signature = build_fn_signature(ctx, node, name);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_rust(ctx, body, sym_id, relations);
    }
}

fn extract_impl(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    _scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    // impl Type { ... } or impl Trait for Type { ... }
    let type_name = node.child_by_field_name("type")
        .map(|n| ctx.node_text(n).to_string())
        .unwrap_or_default();

    if type_name.is_empty() {
        return;
    }

    // Clean generic params from type name
    let clean_name = type_name.split('<').next().unwrap_or(&type_name).trim();

    // If impl Trait for Type, create Implements relation
    if let Some(trait_node) = node.child_by_field_name("trait") {
        let trait_name = ctx.node_text(trait_node);
        let clean_trait = trait_name.split('<').next().unwrap_or(trait_name).trim();
        if !clean_trait.is_empty() {
            // Find the type's symbol ID (best effort — use synthetic if not found)
            let type_qname = qualified_name(scope, clean_name);
            let type_id = SymbolId::generate("", "", &type_qname, 0, 0);
            let trait_id = SymbolId::generate("", "", clean_trait, 0, 0);
            relations.push(CodeRelation {
                source_id: type_id,
                target_id: trait_id,
                kind: RelationKind::Implements,
                file_path: PathBuf::from(ctx.file_path),
                line: node.start_position().row as u32,
                confidence: RelationKind::Implements.default_confidence(),
            });
        }
    }

    // Extract methods within impl body, scoped under the type name
    if let Some(body) = node.child_by_field_name("body") {
        let impl_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(clean_name)).collect();
        // Find the actual type symbol ID to use as parent (best effort)
        let type_qname = qualified_name(scope, clean_name);
        let parent_id = SymbolId::generate("", "", &type_qname, 0, 0);

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_item" => {
                    extract_function(ctx, child, &impl_scope, parent_id, true, symbols, relations);
                }
                "const_item" => {
                    extract_const(ctx, child, &impl_scope, parent_id, symbols, relations);
                }
                "type_item" => {
                    extract_type_alias(ctx, child, &impl_scope, parent_id, symbols, relations);
                }
                _ => {}
            }
        }
    }
}

fn extract_const(
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

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Constant,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: None,
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);
}

fn extract_type_alias(
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

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::TypeAlias,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: None,
        doc_comment: extract_doc_comment(ctx, node),
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);
}

fn extract_use(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    // use crate::foo::bar; / use std::collections::HashMap;
    if let Some(arg) = node.child_by_field_name("argument") {
        let path_text = ctx.node_text(arg);
        // Normalize Rust path to canonical
        let canonical = QualifiedName::normalize(path_text, Language::Rust);
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

fn extract_calls_rust(
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
                    let canonical = QualifiedName::normalize(callee, Language::Rust);
                    let target_id = SymbolId::generate("", "", &canonical, 0, 0);
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
        if child.kind() != "function_item" && child.kind() != "closure_expression" {
            extract_calls_rust(ctx, child, caller_id, relations);
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

fn module_name_from_path(file_path: &str) -> String {
    let p = std::path::Path::new(file_path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    if stem == "lib" || stem == "main" || stem == "mod" {
        p.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(stem)
            .to_string()
    } else {
        stem.to_string()
    }
}

fn build_fn_signature(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| format!(" -> {}", ctx.node_text(n)))
        .unwrap_or_default();
    format!("fn {name}{params}{return_type}")
}

fn extract_doc_comment(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>) -> Option<String> {
    // Collect preceding doc comments (/// or //!)
    let mut lines = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "line_comment" {
            let text = ctx.node_text(sibling);
            if let Some(stripped) = text.strip_prefix("///") {
                lines.push(stripped.trim().to_string());
            } else if let Some(stripped) = text.strip_prefix("//!") {
                lines.push(stripped.trim().to_string());
            } else {
                break;
            }
        } else if sibling.kind() == "attribute_item" {
            // Skip #[...] attributes between doc comments and the item
            prev = sibling.prev_sibling();
            continue;
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
