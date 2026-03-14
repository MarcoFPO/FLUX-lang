use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::ast::*;

// ---------------------------------------------------------------------------
// RegionError — a single region-validation diagnostic
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RegionError {
    pub error_code: u32,
    pub node_id: String,
    pub violation: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// RegionInfo — metadata about a single R-Node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RegionInfo {
    lifetime: Lifetime,
    parent: Option<String>,
    /// Depth in the region hierarchy: 0 = static root, 1 = first scoped child, etc.
    /// `None` when depth cannot be computed (e.g. due to cycles or missing parents).
    depth: Option<u32>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Validate all region-related invariants of a parsed FTL program.
///
/// Returns an empty vector when the program is well-formed with respect to
/// region semantics.  The checks are intentionally *conservative*: in
/// ambiguous situations no error is emitted (false negatives are acceptable,
/// false positives are not).
pub fn check_regions(program: &Program) -> Vec<RegionError> {
    let mut errors = Vec::new();
    let region_index = build_region_index(program);

    check_region_hierarchy(&region_index, &mut errors);
    check_region_references(program, &region_index, &mut errors);
    check_region_escapes(program, &region_index, &mut errors);

    errors
}

// ---------------------------------------------------------------------------
// Step 1 — Build region index
// ---------------------------------------------------------------------------

fn build_region_index(program: &Program) -> HashMap<String, RegionInfo> {
    let mut index: HashMap<String, RegionInfo> = HashMap::new();

    for r in &program.regions {
        let id = r.id.as_str().to_owned();
        let parent = r.parent.as_ref().map(|p| p.as_str().to_owned());
        index.insert(
            id,
            RegionInfo {
                lifetime: r.lifetime.clone(),
                parent,
                depth: None, // resolved below
            },
        );
    }

    // Resolve depths.  Static regions without a parent have depth 0.
    // Scoped regions inherit depth = parent.depth + 1 if the parent exists
    // and itself has a resolved depth.  We iterate until no more progress is
    // made (handles arbitrary nesting).
    loop {
        let mut progress = false;

        // Collect updates to avoid borrow-conflict on `index`.
        let updates: Vec<(String, u32)> = index
            .iter()
            .filter(|(_, info)| info.depth.is_none())
            .filter_map(|(id, info)| {
                match info.lifetime {
                    Lifetime::Static => {
                        // Static regions without a parent are roots.
                        if info.parent.is_none() {
                            Some((id.clone(), 0))
                        } else {
                            // Static with parent is an error (6003), but we still
                            // try to assign depth 0 for downstream checks.
                            Some((id.clone(), 0))
                        }
                    }
                    Lifetime::Scoped => {
                        if let Some(ref parent_id) = info.parent
                            && let Some(parent_info) = index.get(parent_id)
                            && let Some(parent_depth) = parent_info.depth
                        {
                            return Some((id.clone(), parent_depth + 1));
                        }
                        // Scoped without parent (6004) or parent not yet resolved
                        // — cannot compute depth.
                        None
                    }
                }
            })
            .collect();

        for (id, d) in updates {
            if let Some(info) = index.get_mut(&id) {
                info.depth = Some(d);
                progress = true;
            }
        }

        if !progress {
            break;
        }
    }

    index
}

// ---------------------------------------------------------------------------
// Step 2 — Region hierarchy checks (6001-6004)
// ---------------------------------------------------------------------------

fn check_region_hierarchy(index: &HashMap<String, RegionInfo>, errors: &mut Vec<RegionError>) {
    for (id, info) in index {
        // 6003: static region must NOT have a parent.
        if matches!(info.lifetime, Lifetime::Static) && info.parent.is_some() {
            errors.push(RegionError {
                error_code: 6003,
                node_id: id.clone(),
                violation: "STATIC_WITH_PARENT".into(),
                message: format!(
                    "Static region {} must not have a parent (has parent {})",
                    id,
                    info.parent.as_deref().unwrap_or("?")
                ),
                suggestion: Some("Remove the parent field or change lifetime to scoped".into()),
            });
        }

        // 6004: scoped region MUST have a parent.
        if matches!(info.lifetime, Lifetime::Scoped) && info.parent.is_none() {
            errors.push(RegionError {
                error_code: 6004,
                node_id: id.clone(),
                violation: "SCOPED_WITHOUT_PARENT".into(),
                message: format!("Scoped region {} must have a parent", id),
                suggestion: Some(
                    "Add a parent field pointing to an enclosing region".into(),
                ),
            });
        }

        // 6001: parent references a non-existent region.
        if let Some(ref parent_id) = info.parent
            && !index.contains_key(parent_id)
        {
            errors.push(RegionError {
                error_code: 6001,
                node_id: id.clone(),
                violation: "REGION_PARENT_NOT_FOUND".into(),
                message: format!(
                    "Region {} references non-existent parent {}",
                    id, parent_id
                ),
                suggestion: Some(format!("Define {} or fix the parent reference", parent_id)),
            });
        }
    }

    // 6002: cycle detection — follow parent chain and look for repetitions.
    for id in index.keys() {
        let mut visited = HashSet::new();
        let mut current = id.clone();
        loop {
            if !visited.insert(current.clone()) {
                // We have seen `current` before — cycle detected.
                errors.push(RegionError {
                    error_code: 6002,
                    node_id: id.clone(),
                    violation: "REGION_CYCLE".into(),
                    message: format!(
                        "Region {} is part of a cyclic parent chain",
                        id
                    ),
                    suggestion: Some("Break the cycle by changing one of the parent references".into()),
                });
                break;
            }
            match index.get(&current).and_then(|info| info.parent.clone()) {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 3 — Region reference checks (6005)
// ---------------------------------------------------------------------------

fn check_region_references(
    program: &Program,
    index: &HashMap<String, RegionInfo>,
    errors: &mut Vec<RegionError>,
) {
    // M-Nodes: alloc references a region.
    for m in &program.memories {
        if let MemoryOp::Alloc { ref region, .. } = m.op {
            let region_id = region.as_str();
            if !index.contains_key(region_id) {
                errors.push(RegionError {
                    error_code: 6005,
                    node_id: m.id.as_str().to_owned(),
                    violation: "INVALID_REGION_REF".into(),
                    message: format!(
                        "Memory node {} references non-existent region {}",
                        m.id, region_id
                    ),
                    suggestion: Some(format!("Define {} or fix the region reference", region_id)),
                });
            }
        }
    }

    // C-Nodes: const with region, const_bytes with region, generic with region.
    for c in &program.computes {
        let region_ref: Option<&NodeRef> = match &c.op {
            ComputeOp::Const { region, .. } => region.as_ref(),
            ComputeOp::ConstBytes { region, .. } => Some(region),
            ComputeOp::Generic { region, .. } => region.as_ref(),
            _ => None,
        };

        if let Some(region) = region_ref {
            let region_id = region.as_str();
            if !index.contains_key(region_id) {
                errors.push(RegionError {
                    error_code: 6005,
                    node_id: c.id.as_str().to_owned(),
                    violation: "INVALID_REGION_REF".into(),
                    message: format!(
                        "Compute node {} references non-existent region {}",
                        c.id, region_id
                    ),
                    suggestion: Some(format!("Define {} or fix the region reference", region_id)),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 4 — Region escape checks (6006)
// ---------------------------------------------------------------------------

/// Conservative region-escape detection.
///
/// For `M:store` operations we check whether the target allocation lives in a
/// region that is *longer-lived* (smaller depth) than the value being stored.
/// This is a conservative approximation — we only flag clear violations where
/// both regions are known.
fn check_region_escapes(
    program: &Program,
    region_index: &HashMap<String, RegionInfo>,
    errors: &mut Vec<RegionError>,
) {
    // Build a lookup:  node-id -> region-id   for M:alloc nodes.
    let mut alloc_region: HashMap<String, String> = HashMap::new();
    for m in &program.memories {
        if let MemoryOp::Alloc { ref region, .. } = m.op {
            alloc_region.insert(m.id.as_str().to_owned(), region.as_str().to_owned());
        }
    }

    // Build a lookup: C-node-id -> region-id  for C-nodes that carry a region.
    let mut compute_region: HashMap<String, String> = HashMap::new();
    for c in &program.computes {
        let region_ref: Option<&NodeRef> = match &c.op {
            ComputeOp::Const { region, .. } => region.as_ref(),
            ComputeOp::ConstBytes { region, .. } => Some(region),
            ComputeOp::Generic { region, .. } => region.as_ref(),
            _ => None,
        };
        if let Some(r) = region_ref {
            compute_region.insert(c.id.as_str().to_owned(), r.as_str().to_owned());
        }
    }

    // Resolve the region of a node reference.  For M-nodes we use the alloc
    // region.  For C-nodes we use the compute region.  Returns `None` when the
    // region cannot be determined (conservative: no error).
    let resolve_region = |node_id: &str| -> Option<&String> {
        alloc_region.get(node_id).or_else(|| compute_region.get(node_id))
    };

    let depth_of = |region_id: &str| -> Option<u32> {
        region_index.get(region_id).and_then(|info| info.depth)
    };

    // Check M:store — target must live at least as long as value.
    for m in &program.memories {
        if let MemoryOp::Store {
            ref target,
            ref value,
            ..
        } = m.op
        {
            let target_region = resolve_region(target.as_str());
            let value_region = resolve_region(value.as_str());

            // Only check when both regions are known.
            if let (Some(tr), Some(vr)) = (target_region, value_region)
                && let (Some(target_depth), Some(value_depth)) = (depth_of(tr), depth_of(vr))
                && value_depth > target_depth
            {
                errors.push(RegionError {
                    error_code: 6006,
                    node_id: m.id.as_str().to_owned(),
                    violation: "REGION_ESCAPE".into(),
                    message: format!(
                        "Store {} writes a value from region {} (depth {}) \
                         into target in region {} (depth {}) — \
                         data would escape the shorter-lived region",
                        m.id, vr, value_depth, tr, target_depth
                    ),
                    suggestion: Some(
                        "Move the value to the same or a longer-lived region".into(),
                    ),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (unit)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal Program with given regions and no other nodes.
    fn program_with_regions(regions: Vec<RegionDef>) -> Program {
        Program {
            types: Vec::new(),
            regions,
            computes: Vec::new(),
            effects: Vec::new(),
            controls: Vec::new(),
            contracts: Vec::new(),
            memories: Vec::new(),
            externs: Vec::new(),
            entry: NodeRef::new("K:f1"),
        }
    }

    fn region(id: &str, lifetime: Lifetime, parent: Option<&str>) -> RegionDef {
        RegionDef {
            id: NodeRef::new(id),
            lifetime,
            parent: parent.map(|p| NodeRef::new(p)),
        }
    }

    #[test]
    fn valid_static_region() {
        let prog = program_with_regions(vec![region("R:b1", Lifetime::Static, None)]);
        let errs = check_regions(&prog);
        assert!(errs.is_empty(), "static root should be valid: {errs:?}");
    }

    #[test]
    fn valid_scoped_with_parent() {
        let prog = program_with_regions(vec![
            region("R:b1", Lifetime::Static, None),
            region("R:b2", Lifetime::Scoped, Some("R:b1")),
        ]);
        let errs = check_regions(&prog);
        assert!(errs.is_empty(), "scoped with parent should be valid: {errs:?}");
    }

    #[test]
    fn error_6001_parent_not_found() {
        let prog = program_with_regions(vec![
            region("R:b1", Lifetime::Scoped, Some("R:b99")),
        ]);
        let errs = check_regions(&prog);
        assert!(
            errs.iter().any(|e| e.error_code == 6001),
            "expected 6001: {errs:?}"
        );
    }

    #[test]
    fn error_6002_cycle() {
        let prog = program_with_regions(vec![
            region("R:b1", Lifetime::Scoped, Some("R:b2")),
            region("R:b2", Lifetime::Scoped, Some("R:b1")),
        ]);
        let errs = check_regions(&prog);
        assert!(
            errs.iter().any(|e| e.error_code == 6002),
            "expected 6002: {errs:?}"
        );
    }

    #[test]
    fn error_6003_static_with_parent() {
        let prog = program_with_regions(vec![
            region("R:b1", Lifetime::Static, None),
            region("R:b2", Lifetime::Static, Some("R:b1")),
        ]);
        let errs = check_regions(&prog);
        assert!(
            errs.iter().any(|e| e.error_code == 6003),
            "expected 6003: {errs:?}"
        );
    }

    #[test]
    fn error_6004_scoped_without_parent() {
        let prog = program_with_regions(vec![
            region("R:b1", Lifetime::Scoped, None),
        ]);
        let errs = check_regions(&prog);
        assert!(
            errs.iter().any(|e| e.error_code == 6004),
            "expected 6004: {errs:?}"
        );
    }
}
