use std::path::PathBuf;

use oc_core::{CodeRelation, CodeSymbol, QualifiedName, RelationKind, SymbolId, SymbolKind};

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
            "class_declaration" => {
                extract_class(ctx, child, scope, scope_id, symbols, relations);
            }
            "interface_declaration" => {
                extract_interface(ctx, child, scope, scope_id, symbols, relations);
            }
            "enum_declaration" => {
                extract_enum(ctx, child, scope, scope_id, symbols, relations);
            }
            "import_declaration" => {
                extract_import(ctx, child, scope_id, relations);
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
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Class,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("class {name}")),
        doc_comment: extract_javadoc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    // Extract extends/implements
    extract_class_heritage(ctx, node, sym_id, relations);

    // Recurse into class body
    if let Some(body) = node.child_by_field_name("body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_class_body(ctx, body, &new_scope, sym_id, symbols, relations);
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
        ctx.repo_id, ctx.file_path, &qname,
        node.start_byte(), node.end_byte(),
    );

    symbols.push(CodeSymbol {
        id: sym_id,
        name: name.to_string(),
        qualified_name: qname.clone(),
        kind: SymbolKind::Interface,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("interface {name}")),
        doc_comment: extract_javadoc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    // Interface extends
    extract_extends(ctx, node, sym_id, RelationKind::Inherits, relations);

    // Methods in interface body
    if let Some(body) = node.child_by_field_name("body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_class_body(ctx, body, &new_scope, sym_id, symbols, relations);
    }
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
        qualified_name: qname.clone(),
        kind: SymbolKind::Enum,
        language: ctx.language,
        file_path: PathBuf::from(ctx.file_path),
        byte_range: node.start_byte()..node.end_byte(),
        line_range: node.start_position().row as u32..node.end_position().row as u32 + 1,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_javadoc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, scope_id, sym_id, node, relations);

    // Enum can have methods in its body
    if let Some(body) = node.child_by_field_name("body") {
        let new_scope: Vec<&str> = scope.iter().copied().chain(std::iter::once(name)).collect();
        extract_enum_body(ctx, body, &new_scope, sym_id, symbols, relations);
    }
}

fn extract_class_body(
    ctx: &VisitorContext<'_>,
    body: tree_sitter::Node<'_>,
    scope: &[&str],
    class_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                extract_method(ctx, child, scope, class_id, symbols, relations);
            }
            "constructor_declaration" => {
                extract_method(ctx, child, scope, class_id, symbols, relations);
            }
            "class_declaration" => {
                extract_class(ctx, child, scope, class_id, symbols, relations);
            }
            "interface_declaration" => {
                extract_interface(ctx, child, scope, class_id, symbols, relations);
            }
            "enum_declaration" => {
                extract_enum(ctx, child, scope, class_id, symbols, relations);
            }
            "field_declaration" => {
                extract_field(ctx, child, scope, class_id, symbols, relations);
            }
            _ => {}
        }
    }
}

fn extract_enum_body(
    ctx: &VisitorContext<'_>,
    body: tree_sitter::Node<'_>,
    scope: &[&str],
    enum_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_body_declarations" => {
                let mut inner = child.walk();
                for decl in child.children(&mut inner) {
                    if decl.kind() == "method_declaration" || decl.kind() == "constructor_declaration" {
                        extract_method(ctx, decl, scope, enum_id, symbols, relations);
                    }
                }
            }
            "enum_constant" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = ctx.node_text(name_node);
                    if !name.is_empty() {
                        let qname = qualified_name(scope, name);
                        let sym_id = SymbolId::generate(
                            ctx.repo_id, ctx.file_path, &qname,
                            child.start_byte(), child.end_byte(),
                        );
                        symbols.push(CodeSymbol {
                            id: sym_id,
                            name: name.to_string(),
                            qualified_name: qname,
                            kind: SymbolKind::Constant,
                            language: ctx.language,
                            file_path: PathBuf::from(ctx.file_path),
                            byte_range: child.start_byte()..child.end_byte(),
                            line_range: child.start_position().row as u32..child.end_position().row as u32 + 1,
                            signature: None,
                            doc_comment: None,
                            body_text: None,
        body_hash: ctx.body_hash(child),
                        });
                        push_contains(ctx, enum_id, sym_id, child, relations);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_method(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    class_id: SymbolId,
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

    let signature = build_method_signature(ctx, node, name);

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
        doc_comment: extract_javadoc(ctx, node),
        body_text: None,
        body_hash: ctx.body_hash(node),
    });

    push_contains(ctx, class_id, sym_id, node, relations);

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_java(ctx, body, sym_id, relations);
    }
}

fn extract_field(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope: &[&str],
    class_id: SymbolId,
    symbols: &mut Vec<CodeSymbol>,
    relations: &mut Vec<CodeRelation>,
) {
    // Inspect modifiers via AST nodes instead of string matching
    let mut has_static = false;
    let mut has_final = false;
    let mut mod_cursor = node.walk();
    for child in node.children(&mut mod_cursor) {
        if child.kind() == "modifiers" {
            let mut inner = child.walk();
            for modifier in child.children(&mut inner) {
                match modifier.kind() {
                    "static" => has_static = true,
                    "final" => has_final = true,
                    _ => {}
                }
            }
        }
    }
    let is_constant = has_static && has_final;

    // Find the variable declarator — extract both constants and regular fields
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = ctx.node_text(name_node);
                if !name.is_empty() {
                    let kind = if is_constant {
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
                    push_contains(ctx, class_id, sym_id, child, relations);
                }
            }
        }
    }
}

fn extract_class_heritage(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    class_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    extract_extends(ctx, node, class_id, RelationKind::Inherits, relations);

    // implements
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "super_interfaces" || child.kind() == "interfaces" {
            extract_type_list(ctx, child, class_id, RelationKind::Implements, relations);
        }
    }
}

fn extract_extends(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    type_id: SymbolId,
    kind: RelationKind,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "superclass" || child.kind() == "extends_interfaces" {
            extract_type_list(ctx, child, type_id, kind, relations);
        }
    }
}

fn extract_type_list(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    source_id: SymbolId,
    kind: RelationKind,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" || child.kind() == "generic_type" {
            let type_name = if child.kind() == "generic_type" {
                child
                    .child(0)
                    .map(|n| ctx.node_text(n))
                    .unwrap_or("")
            } else {
                ctx.node_text(child)
            };
            if !type_name.is_empty() {
                let target_id = SymbolId::generate("", "", type_name, 0, 0);
                relations.push(CodeRelation {
                    source_id,
                    target_id,
                    kind,
                    file_path: PathBuf::from(ctx.file_path),
                    line: child.start_position().row as u32,
                    confidence: kind.default_confidence(),
                });
            }
        } else if child.kind() == "type_list" || child.kind() == "interface_type_list" {
            extract_type_list(ctx, child, source_id, kind, relations);
        }
    }
}

fn extract_import(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    scope_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    // import java.util.List;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "scoped_identifier" {
            let import_path = ctx.node_text(child);
            if !import_path.is_empty() {
                let target_id = SymbolId::generate("", "", import_path, 0, 0);
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
}

fn extract_calls_java(
    ctx: &VisitorContext<'_>,
    node: tree_sitter::Node<'_>,
    caller_id: SymbolId,
    relations: &mut Vec<CodeRelation>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_invocation" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let callee = ctx.node_text(name_node);
                if !callee.is_empty() {
                    // Include object if present: obj.method
                    let full_name = child
                        .child_by_field_name("object")
                        .map(|obj| format!("{}.{}", ctx.node_text(obj), callee))
                        .unwrap_or_else(|| callee.to_string());
                    let target_id = SymbolId::generate("", "", &full_name, 0, 0);
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
        } else if child.kind() == "object_creation_expression" {
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_name = ctx.node_text(type_node);
                if !type_name.is_empty() {
                    let target_id = SymbolId::generate("", "", type_name, 0, 0);
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
        if child.kind() != "class_declaration" && child.kind() != "lambda_expression" {
            extract_calls_java(ctx, child, caller_id, relations);
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
        if child.kind() == "package_declaration" {
            let mut inner = child.walk();
            for pkg_child in child.children(&mut inner) {
                if pkg_child.kind() == "scoped_identifier" || pkg_child.kind() == "identifier" {
                    return ctx.node_text(pkg_child).to_string();
                }
            }
        }
    }
    // Fallback to file name
    let p = std::path::Path::new(ctx.file_path);
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("default")
        .to_string()
}

fn build_method_signature(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|n| ctx.node_text(n))
        .unwrap_or("()");
    let return_type = node
        .child_by_field_name("type")
        .map(|n| format!("{} ", ctx.node_text(n)))
        .unwrap_or_default();
    format!("{return_type}{name}{params}")
}

fn extract_javadoc(ctx: &VisitorContext<'_>, node: tree_sitter::Node<'_>) -> Option<String> {
    // Walk backward past annotations (marker_annotation, annotation) to find Javadoc
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        match sibling.kind() {
            "block_comment" => {
                let text = ctx.node_text(sibling);
                if !text.starts_with("/**") {
                    return None;
                }
                let stripped = text
                    .strip_prefix("/**")
                    .and_then(|s| s.strip_suffix("*/"))
                    .unwrap_or(text);
                let cleaned: Vec<&str> = stripped
                    .lines()
                    .map(|line| line.trim().trim_start_matches('*').trim())
                    .filter(|line| !line.is_empty())
                    .collect();
                if cleaned.is_empty() {
                    return None;
                }
                return Some(cleaned.join("\n"));
            }
            "marker_annotation" | "annotation" | "modifiers" => {
                // Skip annotations between Javadoc and the declaration
                prev = sibling.prev_sibling();
                continue;
            }
            _ => return None,
        }
    }
    None
}
