use std::path::PathBuf;

use oc_core::{
    CodeRelation, CodeSymbol, Language, QualifiedName, RelationKind, SymbolId, SymbolKind,
};

use crate::error::ParserError;
use crate::visitor::{ParseOutput, VisitorContext};

/// Extract symbols and relations from a Python source file.
pub(crate) fn extract(
    ctx: &VisitorContext<'_>,
    tree: &tree_sitter::Tree,
) -> Result<ParseOutput, ParserError> {
    let mut symbols = Vec::new();
    let mut relations = Vec::new();

    let root = tree.root_node();
    let module_name = module_name_from_path(ctx.file_path);
    let module_qname = module_name.clone();

    let module_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &module_qname,
        root.start_byte(),
        root.end_byte(),
    );
    symbols.push(CodeSymbol {
        id: module_id,
        name: module_name.clone(),
        qualified_name: module_qname.clone(),
        kind: SymbolKind::Module,
        language: Language::Python,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: root.start_byte()..root.end_byte(),
        line_range: root.start_position().row as u32..root.end_position().row as u32 + 1,
        signature: None,
        doc_comment: None,
        body_hash: ctx.body_hash(root),
    });

    let scope = vec![module_name.as_str()];
    extract_children(ctx, root, &scope, module_id, false, &mut symbols, &mut relations);

    Ok(ParseOutput { symbols, relations })
}

fn extract_children(
    ctx: &VisitorContext<'_>,
    parent: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    in_class: bool,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                extract_class(ctx, child, scope, scope_id, symbols, relations);
            }
            "function_definition" => {
                extract_function(ctx, child, scope, scope_id, in_class, symbols, relations);
            }
            "decorated_definition" => {
                extract_decorated(ctx, child, scope, scope_id, in_class, symbols, relations);
            }
            "import_statement" | "import_from_statement" => {
                extract_import(ctx, child, scope_id, relations);
            }
            "expression_statement" => {
                if let Some(assign) = first_child_of_kind(child, "assignment") {
                    extract_assignment(ctx, assign, scope, scope_id, symbols, relations);
                }
            }
            "assignment" => {
                extract_assignment(ctx, child, scope, scope_id, symbols, relations);
            }
            _ => {}
        }
    }
}

fn extract_class(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let name = match child_by_field(node, "name") {
        Some(n) => ctx.node_text(n),
        None => return,
    };

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &qname,
        node.start_byte(),
        node.end_byte(),
    );

    let signature = build_class_signature(ctx, node, name);
    let doc = extract_docstring(ctx, node);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Class,
        language: Language::Python,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: doc,
        body_hash: ctx.body_hash(node),
    });

    // Contains relation
    relations.push(CodeRelation {
        source_id: scope_id,
        target_id: sym_id,
        kind: RelationKind::Contains,
        file_path: PathBuf::from(ctx.file_path),
        line: node.start_position().row as u32,
        confidence: RelationKind::Contains.default_confidence(),
    });

    // Inherits relations from base classes
    if let Some(args) = child_by_field(node, "superclasses") {
        extract_base_classes(ctx, args, sym_id, relations);
    }

    // Recurse into class body — children are in_class=true
    if let Some(body) = child_by_field(node, "body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_children(ctx, body, &new_scope, sym_id, true, symbols, relations);
    }
}

fn extract_function(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    in_class: bool,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let name = match child_by_field(node, "name") {
        Some(n) => ctx.node_text(n),
        None => return,
    };

    if name.is_empty() {
        return;
    }

    let kind = if in_class {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &qname,
        node.start_byte(),
        node.end_byte(),
    );

    let signature = build_function_signature(ctx, node, name);
    let doc = extract_docstring(ctx, node);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind,
        language: Language::Python,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: doc,
        body_hash: ctx.body_hash(node),
    });

    // Contains relation
    relations.push(CodeRelation {
        source_id: scope_id,
        target_id: sym_id,
        kind: RelationKind::Contains,
        file_path: PathBuf::from(ctx.file_path),
        line: node.start_position().row as u32,
        confidence: RelationKind::Contains.default_confidence(),
    });

    // Extract calls within the function body
    if let Some(body) = child_by_field(node, "body") {
        extract_calls(ctx, body, sym_id, relations);

        // Recurse for nested definitions (functions/classes) including those inside
        // control-flow blocks (if/for/while/try/with)
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_nested_recursive(ctx, body, &new_scope, sym_id, symbols, relations);
    }
}

fn extract_decorated(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    in_class: bool,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    if let Some(definition) = child_by_field(node, "definition") {
        match definition.kind() {
            "class_definition" => {
                extract_class(ctx, definition, scope, scope_id, symbols, relations);
            }
            "function_definition" => {
                extract_function(
                    ctx, definition, scope, scope_id, in_class, symbols, relations,
                );
            }
            _ => {}
        }
    }
}

fn extract_assignment(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let lhs = match child_by_field(node, "left") {
        Some(n) => n,
        None => return,
    };

    if lhs.kind() != "identifier" {
        return;
    }

    let name = ctx.node_text(lhs);
    if name.is_empty() {
        return;
    }

    let kind = if is_constant_name(name) {
        SymbolKind::Constant
    } else {
        SymbolKind::Variable
    };

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &qname,
        node.start_byte(),
        node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname,
        kind,
        language: Language::Python,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: None,
        doc_comment: None,
        body_hash: ctx.body_hash(node),
    });

    relations.push(CodeRelation {
        source_id: scope_id,
        target_id: sym_id,
        kind: RelationKind::Contains,
        file_path: PathBuf::from(ctx.file_path),
        line: node.start_position().row as u32,
        confidence: RelationKind::Contains.default_confidence(),
    });
}

fn extract_import(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    match node.kind() {
        "import_statement" => {
            // import foo, import foo.bar, import numpy as np
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        let module_name = ctx.node_text(child);
                        push_import(ctx, scope_id, module_name, node.start_position().row as u32, relations);
                    }
                    "aliased_import" => {
                        // import numpy as np — extract the original module name
                        if let Some(name_node) = child_by_field(child, "name") {
                            let module_name = ctx.node_text(name_node);
                            push_import(ctx, scope_id, module_name, node.start_position().row as u32, relations);
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            // from foo import bar, baz / from foo import * / from foo import bar as b
            let module_name = child_by_field(node, "module_name")
                .map(|n| ctx.node_text(n).to_string())
                .unwrap_or_default();

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        let text = ctx.node_text(child);
                        // Skip the module name portion itself
                        if text == module_name {
                            continue;
                        }
                        let full_name = format_import_name(&module_name, text);
                        push_import(ctx, scope_id, &full_name, node.start_position().row as u32, relations);
                    }
                    "aliased_import" => {
                        if let Some(name_node) = child_by_field(child, "name") {
                            let imported = ctx.node_text(name_node);
                            if !imported.is_empty() {
                                let full_name = format_import_name(&module_name, imported);
                                push_import(ctx, scope_id, &full_name, node.start_position().row as u32, relations);
                            }
                        }
                    }
                    "wildcard_import" => {
                        // from foo import * — import the module itself
                        if !module_name.is_empty() {
                            push_import(ctx, scope_id, &module_name, node.start_position().row as u32, relations);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn push_import(
    ctx: &VisitorContext<'_>,
    scope_id: SymbolId,
    target_name: &str,
    line: u32,
    relations: &mut Vec<CodeRelation>,
) {
    let target_id = SymbolId::generate("", "", target_name, 0, 0);
    relations.push(CodeRelation {
        source_id: scope_id,
        target_id,
        kind: RelationKind::Imports,
        file_path: PathBuf::from(ctx.file_path),
        line,
        confidence: RelationKind::Imports.default_confidence(),
    });
}

fn format_import_name(module_name: &str, imported: &str) -> String {
    if module_name.is_empty() {
        imported.to_string()
    } else {
        format!("{module_name}.{imported}")
    }
}

fn extract_base_classes(
    ctx: &VisitorContext<'_>,
    args_node: tree_sitter::Node<'_>,
    class_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "attribute" => {
                let base_name = ctx.node_text(child);
                let target_id = SymbolId::generate("", "", base_name, 0, 0);
                relations.push(CodeRelation {
                    source_id: class_id,
                    target_id,
                    kind: RelationKind::Inherits,
                    file_path: PathBuf::from(ctx.file_path),
                    line: child.start_position().row as u32,
                    confidence: RelationKind::Inherits.default_confidence(),
                });
            }
            _ => {}
        }
    }
}

fn extract_calls(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    caller_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    traverse_for_calls(ctx, node, caller_id, relations);
}

fn traverse_for_calls(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    caller_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func) = child_by_field(child, "function") {
                let callee_name = ctx.node_text(func);
                if !callee_name.is_empty() {
                    let target_id = SymbolId::generate("", "", callee_name, 0, 0);
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
        // Recurse but skip nested function/class definitions (they have their own scope)
        if child.kind() != "function_definition" && child.kind() != "class_definition" {
            traverse_for_calls(ctx, child, caller_id, relations);
        }
    }
}

/// Recursively find nested function/class definitions, including those inside
/// control-flow blocks (if, for, while, try, with, etc.).
fn extract_nested_recursive(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    parent_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                extract_function(ctx, child, scope, parent_id, false, symbols, relations);
            }
            "class_definition" => {
                extract_class(ctx, child, scope, parent_id, symbols, relations);
            }
            "decorated_definition" => {
                extract_decorated(ctx, child, scope, parent_id, false, symbols, relations);
            }
            // Recurse into compound statements to find nested definitions
            "if_statement" | "for_statement" | "while_statement" | "try_statement"
            | "with_statement" | "elif_clause" | "else_clause" | "except_clause"
            | "finally_clause" | "block" => {
                extract_nested_recursive(ctx, child, scope, parent_id, symbols, relations);
            }
            _ => {}
        }
    }
}

// --- Helpers ---

fn child_by_field<'a>(
    node: tree_sitter::Node<'a>,
    field: &str,
) -> Option<tree_sitter::Node<'a>> {
    node.child_by_field_name(field)
}

fn first_child_of_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor).find(|c| c.kind() == kind);
    result
}

fn qualified_name(scope: &[&str], name: &str) -> String {
    let mut parts: Vec<&str> = scope.to_vec();
    parts.push(name);
    QualifiedName::join(&parts)
}

fn module_name_from_path(file_path: &str) -> String {
    let p = std::path::Path::new(file_path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    if stem == "__init__" {
        p.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("module")
            .to_string()
    } else {
        stem.to_string()
    }
}

fn is_constant_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        && name.chars().any(|c| c.is_ascii_uppercase())
}

fn build_function_signature(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    name: &str,
) -> String {
    let params = child_by_field(node, "parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let return_type = child_by_field(node, "return_type")
        .map(|n| format!(" -> {}", ctx.node_text(n)))
        .unwrap_or_default();
    format!("def {name}{params}{return_type}")
}

fn build_class_signature(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    name: &str,
) -> String {
    let bases = child_by_field(node, "superclasses")
        .map(|n| ctx.node_text(n))
        .unwrap_or("");
    if bases.is_empty() {
        format!("class {name}")
    } else {
        format!("class {name}{bases}")
    }
}

fn extract_docstring(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>) -> Option<String> {
    let body = child_by_field(node, "body")?;
    let mut cursor = body.walk();
    let first_stmt = body.children(&mut cursor).next()?;

    if first_stmt.kind() != "expression_statement" {
        return None;
    }

    let mut inner_cursor = first_stmt.walk();
    let expr = first_stmt.children(&mut inner_cursor).next()?;

    if expr.kind() != "string" {
        return None;
    }

    let text = ctx.node_text(expr);
    let stripped = text
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
        .or_else(|| {
            text.strip_prefix("'''")
                .and_then(|s| s.strip_suffix("'''"))
        })
        .or_else(|| {
            text.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
        })
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(text);

    Some(stripped.trim().to_string())
}
