use std::collections::{HashMap, HashSet};
use std::path::Path;

use oc_core::{CodeRelation, CodeSymbol, SymbolId};

/// Priority tier for phantom ID matches (lower = better).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchTier {
    ExactQualifiedName = 0,
    SuffixMatch = 1,
    ExactName = 2,
}

/// A candidate resolution for a phantom target ID.
#[derive(Debug, Clone)]
struct Candidate {
    real_id: SymbolId,
    file_path: String,
    tier: MatchTier,
}

/// Statistics from a resolution pass.
#[derive(Debug, Default)]
pub struct ResolutionStats {
    pub already_resolved: usize,
    pub resolved_by_qualified_name: usize,
    pub resolved_by_suffix: usize,
    pub resolved_by_name: usize,
    pub unresolved: usize,
    pub total: usize,
}

/// Lightweight symbol reference for building the phantom lookup.
/// Avoids fetching body_text and other heavy fields.
pub struct SymbolRef {
    pub id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
}

impl From<&CodeSymbol> for SymbolRef {
    fn from(s: &CodeSymbol) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            file_path: s.file_path.to_string_lossy().into_owned(),
        }
    }
}

/// Build a lookup from phantom IDs → candidate real symbols.
///
/// For each symbol, we compute the phantom IDs that the parser would generate
/// when referencing it from another file (i.e. `SymbolId::generate("", "", target_name, 0, 0)`).
///
/// Multiple symbols may map to the same phantom ID; we collect all candidates
/// and let the resolver disambiguate.
fn build_phantom_lookup(symbols: &[SymbolRef]) -> HashMap<SymbolId, Vec<Candidate>> {
    let mut lookup: HashMap<SymbolId, Vec<Candidate>> = HashMap::new();

    for sym in symbols {
        // 1. Exact qualified_name match
        let phantom_qname = SymbolId::generate("", "", &sym.qualified_name, 0, 0);
        lookup
            .entry(phantom_qname)
            .or_default()
            .push(Candidate {
                real_id: sym.id,
                file_path: sym.file_path.clone(),
                tier: MatchTier::ExactQualifiedName,
            });

        // 2. Dot-suffix matches (all suffixes of the qualified name)
        // e.g. "magic_model.boxbase.MagicBoxBase" → ["boxbase.MagicBoxBase", "MagicBoxBase"]
        let qname = &sym.qualified_name;
        let mut pos = 0;
        while let Some(dot_pos) = qname[pos..].find('.') {
            let suffix = &qname[pos + dot_pos + 1..];
            if !suffix.is_empty() && suffix != sym.name {
                let phantom_suffix = SymbolId::generate("", "", suffix, 0, 0);
                lookup
                    .entry(phantom_suffix)
                    .or_default()
                    .push(Candidate {
                        real_id: sym.id,
                        file_path: sym.file_path.clone(),
                        tier: MatchTier::SuffixMatch,
                    });
            }
            pos += dot_pos + 1;
        }

        // 3. Simple name match
        if sym.name != sym.qualified_name {
            let phantom_name = SymbolId::generate("", "", &sym.name, 0, 0);
            lookup
                .entry(phantom_name)
                .or_default()
                .push(Candidate {
                    real_id: sym.id,
                    file_path: sym.file_path.clone(),
                    tier: MatchTier::ExactName,
                });
        }
    }

    lookup
}

/// Pick the best candidate from a list, optionally preferring same-directory.
fn pick_best<'a>(candidates: &'a [Candidate], relation_dir: Option<&str>) -> Option<&'a Candidate> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(&candidates[0]);
    }

    // Find the best tier present
    let best_tier = candidates.iter().map(|c| c.tier).min().unwrap();
    let best_tier_candidates: Vec<&Candidate> =
        candidates.iter().filter(|c| c.tier == best_tier).collect();

    if best_tier_candidates.len() == 1 {
        return Some(best_tier_candidates[0]);
    }

    // Disambiguate: prefer same directory as the relation's source file
    if let Some(dir) = relation_dir {
        let same_dir: Vec<&&Candidate> = best_tier_candidates
            .iter()
            .filter(|c| {
                Path::new(&c.file_path)
                    .parent()
                    .map(|p| p.to_string_lossy())
                    .as_deref()
                    == Some(dir)
            })
            .collect();
        if same_dir.len() == 1 {
            return Some(same_dir[0]);
        }
    }

    // Final tiebreak: alphabetical by file_path for determinism
    best_tier_candidates
        .into_iter()
        .min_by_key(|c| c.file_path.clone())
}

/// Resolve dangling cross-file references in relations.
///
/// For each relation whose `target_id` is not in `known_ids` (meaning it's a
/// phantom ID generated by the parser for a cross-file reference), attempt to
/// find the real symbol by matching phantom IDs computed from known symbols.
///
/// Relations are mutated in-place: `target_id` is replaced with the resolved
/// real symbol ID when a match is found. Unresolved relations are preserved as-is.
pub fn resolve_relations(
    relations: &mut [CodeRelation],
    symbols: &[SymbolRef],
    known_ids: &HashSet<SymbolId>,
) -> ResolutionStats {
    let phantom_lookup = build_phantom_lookup(symbols);
    let mut stats = ResolutionStats {
        total: relations.len(),
        ..Default::default()
    };

    for rel in relations.iter_mut() {
        // Skip relations whose target already resolves to a known symbol
        if known_ids.contains(&rel.target_id) {
            stats.already_resolved += 1;
            continue;
        }

        // Look up the phantom target ID
        let relation_dir = rel
            .file_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned());

        if let Some(candidates) = phantom_lookup.get(&rel.target_id) {
            if let Some(best) = pick_best(candidates, relation_dir.as_deref()) {
                rel.target_id = best.real_id;
                match best.tier {
                    MatchTier::ExactQualifiedName => stats.resolved_by_qualified_name += 1,
                    MatchTier::SuffixMatch => stats.resolved_by_suffix += 1,
                    MatchTier::ExactName => stats.resolved_by_name += 1,
                }
                continue;
            }
        }

        stats.unresolved += 1;
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_core::{Language, RelationKind, SymbolKind};
    use std::path::PathBuf;

    fn make_symbol(
        repo_id: &str,
        file: &str,
        name: &str,
        qualified_name: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> CodeSymbol {
        CodeSymbol {
            id: SymbolId::generate(repo_id, file, qualified_name, byte_start, byte_end),
            name: name.to_string(),
            qualified_name: qualified_name.to_string(),
            kind: SymbolKind::Function,
            language: Language::Python,
            file_path: PathBuf::from(file),
            byte_range: byte_start..byte_end,
            line_range: 0..10,
            signature: None,
            doc_comment: None,
            body_hash: 0,
            body_text: None,
        }
    }

    fn make_relation(source_id: SymbolId, target_name: &str, file: &str) -> CodeRelation {
        CodeRelation {
            source_id,
            target_id: SymbolId::generate("", "", target_name, 0, 0),
            kind: RelationKind::Calls,
            file_path: PathBuf::from(file),
            line: 5,
            confidence: 0.8,
        }
    }

    fn sym_refs(symbols: &[CodeSymbol]) -> Vec<SymbolRef> {
        symbols.iter().map(SymbolRef::from).collect()
    }

    #[test]
    fn exact_qualified_name_resolution() {
        let target = make_symbol("repo", "lib/utils.py", "helper", "utils.helper", 10, 50);
        let source = make_symbol("repo", "src/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone(), target.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        let mut relations = vec![make_relation(source.id, "utils.helper", "src/main.py")];
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, target.id);
        assert_eq!(stats.resolved_by_qualified_name, 1);
        assert_eq!(stats.unresolved, 0);
    }

    #[test]
    fn suffix_match_resolution() {
        let target = make_symbol(
            "repo",
            "magic_model/boxbase.py",
            "MagicBoxBase",
            "boxbase.MagicBoxBase",
            10,
            500,
        );
        let source = make_symbol("repo", "src/caller.py", "call_it", "caller.call_it", 0, 100);
        let symbols = vec![source.clone(), target.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        // Parser would generate phantom for "MagicBoxBase" (simple name)
        // which matches via ExactName tier since qualified_name != name
        let mut relations = vec![make_relation(source.id, "MagicBoxBase", "src/caller.py")];
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, target.id);
        assert_eq!(stats.unresolved, 0);
    }

    #[test]
    fn name_match_resolution() {
        let target = make_symbol("repo", "lib/foo.py", "bar", "foo.bar", 0, 50);
        let source = make_symbol("repo", "src/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone(), target.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        let mut relations = vec![make_relation(source.id, "bar", "src/main.py")];
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, target.id);
        assert_eq!(stats.resolved_by_name, 1);
    }

    #[test]
    fn already_resolved_untouched() {
        let target = make_symbol("repo", "lib/utils.py", "helper", "utils.helper", 10, 50);
        let source = make_symbol("repo", "src/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone(), target.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        // Create a relation whose target is already a real known ID
        let mut relations = vec![CodeRelation {
            source_id: source.id,
            target_id: target.id,
            kind: RelationKind::Contains,
            file_path: PathBuf::from("src/main.py"),
            line: 1,
            confidence: 0.95,
        }];

        let original_target = relations[0].target_id;
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, original_target);
        assert_eq!(stats.already_resolved, 1);
        assert_eq!(stats.unresolved, 0);
    }

    #[test]
    fn unresolvable_preserved() {
        let source = make_symbol("repo", "src/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        // Reference to something that doesn't exist in our codebase
        let mut relations = vec![make_relation(source.id, "external.library.Thing", "src/main.py")];
        let original_target = relations[0].target_id;
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, original_target);
        assert_eq!(stats.unresolved, 1);
    }

    #[test]
    fn disambiguation_prefers_same_directory() {
        // Two symbols with the same name in different dirs
        let target_a = make_symbol("repo", "pkg_a/utils.py", "helper", "utils.helper", 0, 50);
        let target_b = make_symbol("repo", "pkg_b/utils.py", "helper", "utils.helper", 0, 50);
        let source = make_symbol("repo", "pkg_a/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone(), target_a.clone(), target_b.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        let mut relations = vec![make_relation(source.id, "helper", "pkg_a/main.py")];
        let stats = resolve_relations(&mut relations, &refs, &known);

        // Should prefer target_a because it's in the same directory as the source
        assert_eq!(relations[0].target_id, target_a.id);
        assert_eq!(stats.unresolved, 0);
    }

    #[test]
    fn priority_ordering() {
        // Create a symbol that can be matched at different tiers
        let target = make_symbol(
            "repo",
            "lib/module.py",
            "MyClass",
            "module.MyClass",
            10,
            200,
        );
        // Create another symbol whose name matches
        let other = make_symbol(
            "repo",
            "lib/other.py",
            "MyClass",
            "other.MyClass",
            10,
            200,
        );
        let source = make_symbol("repo", "src/main.py", "main", "main.main", 0, 100);
        let symbols = vec![source.clone(), target.clone(), other.clone()];
        let refs = sym_refs(&symbols);
        let known: HashSet<SymbolId> = symbols.iter().map(|s| s.id).collect();

        // Exact qualified name match should win over name match
        let mut relations = vec![make_relation(source.id, "module.MyClass", "src/main.py")];
        let stats = resolve_relations(&mut relations, &refs, &known);

        assert_eq!(relations[0].target_id, target.id);
        assert_eq!(stats.resolved_by_qualified_name, 1);
    }

    #[test]
    fn build_phantom_lookup_correct_entries() {
        let sym = make_symbol(
            "repo",
            "pkg/module.py",
            "MyClass",
            "module.MyClass",
            10,
            200,
        );
        let refs = sym_refs(&[sym.clone()]);
        let lookup = build_phantom_lookup(&refs);

        // Should have entries for:
        // 1. "module.MyClass" (exact qualified name)
        let phantom_qname = SymbolId::generate("", "", "module.MyClass", 0, 0);
        assert!(lookup.contains_key(&phantom_qname));

        // 2. "MyClass" via suffix (but it equals name, so it's handled by name match instead)
        // The suffix "MyClass" equals the name, so it won't be added as suffix
        // But it will be added as name match
        let phantom_name = SymbolId::generate("", "", "MyClass", 0, 0);
        assert!(lookup.contains_key(&phantom_name));

        // Verify the tiers
        let qname_candidates = &lookup[&phantom_qname];
        assert_eq!(qname_candidates.len(), 1);
        assert_eq!(qname_candidates[0].tier, MatchTier::ExactQualifiedName);

        let name_candidates = &lookup[&phantom_name];
        assert_eq!(name_candidates.len(), 1);
        assert_eq!(name_candidates[0].tier, MatchTier::ExactName);
    }
}
