use std::path::PathBuf;

use oc_core::{CodeRelation, CodeSymbol, QualifiedName, RelationKind, SymbolId, SymbolKind};

use crate::error::ParserError;
use crate::visitor::{ParseOutput, VisitorContext};

/// Extract symbols and relations from a TypeScript or JavaScript source file.
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
        body_text: None,
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
            "class_declaration" => {
                extract_class(ctx, child, scope, scope_id, symbols, relations);
            }
            "abstract_class_declaration" => {
                extract_class(ctx, child, scope, scope_id, symbols, relations);
            }
            "function_declaration" => {
                extract_function(ctx, child, scope, scope_id, in_class, symbols, relations);
            }
            "method_definition" => {
                extract_method(ctx, child, scope, scope_id, symbols, relations);
            }
            "interface_declaration" => {
                extract_interface(ctx, child, scope, scope_id, symbols, relations);
            }
            "type_alias_declaration" => {
                extract_type_alias(ctx, child, scope, scope_id, symbols, relations);
            }
            "enum_declaration" => {
                extract_enum(ctx, child, scope, scope_id, symbols, relations);
            }
            "import_statement" => {
                extract_import(ctx, child, scope_id, relations);
            }
            "export_statement" => {
                // Export statements wrap the actual declaration
                extract_export(ctx, child, scope, scope_id, in_class, symbols, relations);
            }
            "lexical_declaration" => {
                // const/let declarations
                extract_lexical_declaration(ctx, child, scope, scope_id, symbols, relations);
            }
            "variable_declaration" => {
                // var declarations
                extract_var_declaration(ctx, child, scope, scope_id, symbols, relations);
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
    let name = match node.child_by_field_name("name") {
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

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Class,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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

    // Heritage: extends and implements
    extract_heritage(ctx, node, sym_id, relations);

    // Recurse into class body
    if let Some(body) = node.child_by_field_name("body") {
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
    let name = match node.child_by_field_name("name") {
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
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls(ctx, body, sym_id, relations);
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_nested_recursive(ctx, body, &new_scope, sym_id, symbols, relations);
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

    let qname = qualified_name(scope, name);
    let sym_id = SymbolId::generate(
        ctx.repo_id,
        ctx.file_path,
        &qname,
        node.start_byte(),
        node.end_byte(),
    );

    let signature = build_method_signature(ctx, node, name);

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Method,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(signature),
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls(ctx, body, sym_id, relations);
    }
}

fn extract_interface(
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
        kind: SymbolKind::Interface,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("interface {name}")),
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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

    // Interface can extend other interfaces
    extract_heritage(ctx, node, sym_id, relations);
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
        kind: SymbolKind::TypeAlias,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: None,
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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
        kind: SymbolKind::Enum,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_jsdoc(ctx, node),
        body_text: None,
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
    // import { a, b } from 'module'
    // import * as ns from 'module'
    // import defaultExport from 'module'
    if let Some(source_node) = node.child_by_field_name("source") {
        let module_path = ctx.node_text(source_node);
        // Strip quotes
        let module_name = module_path
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();

        if !module_name.is_empty() {
            let target_id = SymbolId::generate("", "", &module_name, 0, 0);
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

fn extract_export(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    in_class: bool,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    // export statement wraps a declaration or re-exports
    // For re-exports: export { x } from 'module' — treat as Imports
    if let Some(source_node) = node.child_by_field_name("source") {
        let module_path = ctx.node_text(source_node);
        let module_name = module_path
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();
        if !module_name.is_empty() {
            let target_id = SymbolId::generate("", "", &module_name, 0, 0);
            relations.push(CodeRelation {
                source_id: scope_id,
                target_id,
                kind: RelationKind::Imports,
                file_path: PathBuf::from(ctx.file_path),
                line: node.start_position().row as u32,
                confidence: RelationKind::Imports.default_confidence(),
            });
        }
        return;
    }

    // export wraps a declaration — extract the inner declaration
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "abstract_class_declaration" => {
                extract_class(ctx, child, scope, scope_id, symbols, relations);
            }
            "function_declaration" => {
                extract_function(ctx, child, scope, scope_id, in_class, symbols, relations);
            }
            "interface_declaration" => {
                extract_interface(ctx, child, scope, scope_id, symbols, relations);
            }
            "type_alias_declaration" => {
                extract_type_alias(ctx, child, scope, scope_id, symbols, relations);
            }
            "enum_declaration" => {
                extract_enum(ctx, child, scope, scope_id, symbols, relations);
            }
            "lexical_declaration" => {
                extract_lexical_declaration(ctx, child, scope, scope_id, symbols, relations);
            }
            _ => {}
        }
    }
}

fn extract_lexical_declaration(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    // const x = 1; let y = 2;
    // Determine if const (for Constant classification)
    let is_const = ctx.node_text(node).starts_with("const");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            extract_variable_declarator(ctx, child, scope, scope_id, is_const, symbols, relations);
        }
    }
}

fn extract_var_declaration(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            extract_variable_declarator(ctx, child, scope, scope_id, false, symbols, relations);
        }
    }
}

fn extract_variable_declarator(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    scope_id: SymbolId,
    is_const: bool,
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

    // Check if the value is an arrow function or function expression — skip as anonymous
    // unless it's being assigned to a named variable (which gives it a name)
    let value = node.child_by_field_name("value");
    let is_func_value = value
        .map(|v| v.kind() == "arrow_function" || v.kind() == "function")
        .unwrap_or(false);

    let kind = if is_func_value {
        // Named arrow functions / function expressions become Function symbols
        SymbolKind::Function
    } else if is_const {
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
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: None,
        doc_comment: None,
        body_text: None,
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

    // Extract calls from the value expression
    if let Some(val) = value {
        extract_calls(ctx, val, sym_id, relations);
    }
}

fn extract_heritage(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    class_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    // Look for extends_clause and implements_clause in class_heritage
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" => {
                let mut inner = child.walk();
                for heritage in child.children(&mut inner) {
                    match heritage.kind() {
                        "extends_clause" => {
                            extract_heritage_names(
                                ctx,
                                heritage,
                                class_id,
                                RelationKind::Inherits,
                                relations,
                            );
                        }
                        "implements_clause" => {
                            extract_heritage_names(
                                ctx,
                                heritage,
                                class_id,
                                RelationKind::Implements,
                                relations,
                            );
                        }
                        _ => {}
                    }
                }
            }
            "extends_clause" => {
                // Interface extends
                extract_heritage_names(ctx, child, class_id, RelationKind::Inherits, relations);
            }
            "extends_type_clause" => {
                extract_heritage_names(ctx, child, class_id, RelationKind::Inherits, relations);
            }
            _ => {}
        }
    }
}

fn extract_heritage_names(
    ctx: &VisitorContext<'_>,
    clause: tree_sitter::Node<'_>,
    class_id: SymbolId,
    kind: RelationKind,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        // Skip keyword tokens like "extends", "implements"
        match child.kind() {
            "identifier" | "type_identifier" | "member_expression" | "generic_type" => {
                let name = if child.kind() == "generic_type" {
                    // Extract just the type name, not the generic params
                    child
                        .child_by_field_name("name")
                        .map(|n| ctx.node_text(n))
                        .unwrap_or("")
                } else {
                    ctx.node_text(child)
                };
                if !name.is_empty() {
                    let target_id = SymbolId::generate("", "", name, 0, 0);
                    relations.push(CodeRelation {
                        source_id: class_id,
                        target_id,
                        kind,
                        file_path: PathBuf::from(ctx.file_path),
                        line: child.start_position().row as u32,
                        confidence: kind.default_confidence(),
                    });
                }
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
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = child.child_by_field_name("function") {
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
        // Also catch "new" expressions as calls
        if child.kind() == "new_expression" {
            if let Some(constructor) = child.child_by_field_name("constructor") {
                let callee_name = ctx.node_text(constructor);
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
        if child.kind() != "function_declaration"
            && child.kind() != "class_declaration"
            && child.kind() != "arrow_function"
        {
            extract_calls(ctx, child, caller_id, relations);
        }
    }
}

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
            "function_declaration" => {
                extract_function(ctx, child, scope, parent_id, false, symbols, relations);
            }
            "class_declaration" | "abstract_class_declaration" => {
                extract_class(ctx, child, scope, parent_id, symbols, relations);
            }
            "if_statement" | "for_statement" | "for_in_statement" | "while_statement"
            | "try_statement" | "switch_statement" | "statement_block" | "catch_clause"
            | "finally_clause" => {
                extract_nested_recursive(ctx, child, scope, parent_id, symbols, relations);
            }
            _ => {}
        }
    }
}

// --- Helpers ---

fn qualified_name(scope: &[&str], name: &str) -> String {
    let mut parts: Vec<&str> = scope.to_vec();
    parts.push(name);
    QualifiedName::join(&parts)
}

fn module_name_from_path(file_path: &str) -> String {
    let p = std::path::Path::new(file_path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    if stem == "index" {
        p.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("module")
            .to_string()
    } else {
        stem.to_string()
    }
}

fn build_class_signature(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>, name: &str) -> String {
    let mut sig = format!("class {name}");
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            sig.push(' ');
            sig.push_str(ctx.node_text(child));
            break;
        }
    }
    sig
}

fn build_function_signature(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    name: &str,
) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| format!(": {}", ctx.node_text(n)))
        .unwrap_or_default();
    format!("function {name}{params}{return_type}")
}

fn build_method_signature(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    name: &str,
) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| format!(": {}", ctx.node_text(n)))
        .unwrap_or_default();
    format!("{name}{params}{return_type}")
}

fn extract_jsdoc(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>) -> Option<String> {
    // Look for a preceding comment node
    let prev = node.prev_sibling()?;
    if prev.kind() != "comment" {
        return None;
    }
    let text = ctx.node_text(prev);
    // Only extract JSDoc-style comments (/** ... */)
    if !text.starts_with("/**") {
        return None;
    }
    let stripped = text
        .strip_prefix("/**")
        .and_then(|s| s.strip_suffix("*/"))
        .unwrap_or(text);
    // Clean up leading * on each line
    let cleaned: Vec<&str> = stripped
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.join("\n"))
}
