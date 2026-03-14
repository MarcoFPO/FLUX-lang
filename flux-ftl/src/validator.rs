use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::ast::*;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeKind {
    Type,
    Region,
    Compute,
    Effect,
    Control,
    Contract,
    Memory,
    Extern,
}

#[derive(Debug, Serialize)]
pub struct ValidationError {
    pub error_code: u32,
    pub node_id: String,
    pub violation: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

pub fn validate(program: &Program) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Phase 1: Build ID index
    let index = build_index(program, &mut errors);

    // Phase 2: Reference checks
    check_references(program, &index, &mut errors);

    // Phase 3: DAG check (cycle detection on operational edges)
    check_dag(program, &index, &mut errors);

    // Phase 4: Entry point validation
    check_entry(program, &index, &mut errors, &mut warnings);

    ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Phase 1: Build ID index
// ---------------------------------------------------------------------------

fn build_index(program: &Program, errors: &mut Vec<ValidationError>) -> HashMap<String, NodeKind> {
    let mut index = HashMap::new();

    let mut insert = |id: &NodeRef, kind: NodeKind, errors: &mut Vec<ValidationError>| {
        let key = id.as_str().to_string();
        use std::collections::hash_map::Entry;
        match index.entry(key) {
            Entry::Occupied(e) => {
                let key = e.key().clone();
                errors.push(ValidationError {
                    error_code: 1001,
                    node_id: key.clone(),
                    violation: "DUPLICATE_ID".to_string(),
                    message: format!("Node {} is defined more than once", key),
                    suggestion: Some(format!(
                        "Rename one of the duplicate {} definitions to a unique ID",
                        key
                    )),
                });
            }
            Entry::Vacant(e) => {
                e.insert(kind);
            }
        }
    };

    for t in &program.types {
        insert(&t.id, NodeKind::Type, errors);
    }
    for r in &program.regions {
        insert(&r.id, NodeKind::Region, errors);
    }
    for c in &program.computes {
        insert(&c.id, NodeKind::Compute, errors);
    }
    for e in &program.effects {
        insert(&e.id, NodeKind::Effect, errors);
    }
    for k in &program.controls {
        insert(&k.id, NodeKind::Control, errors);
    }
    for v in &program.contracts {
        insert(&v.id, NodeKind::Contract, errors);
    }
    for m in &program.memories {
        insert(&m.id, NodeKind::Memory, errors);
    }
    for x in &program.externs {
        insert(&x.id, NodeKind::Extern, errors);
    }

    index
}

// ---------------------------------------------------------------------------
// Phase 2: Reference checks
// ---------------------------------------------------------------------------

/// A reference from one node to another, for error reporting.
struct Ref<'a> {
    /// The node that contains the reference.
    source_id: &'a str,
    /// The referenced node.
    target: &'a NodeRef,
}

/// Returns true if the given string is a valid FTL node ID (has a known prefix
/// followed by a colon, e.g. "C:c1", "T:a1", "M:g3").
fn is_node_id(id: &str) -> bool {
    matches!(
        id.split(':').next(),
        Some("T" | "R" | "C" | "E" | "K" | "V" | "M" | "X")
    ) && id.contains(':')
}

fn check_references(
    program: &Program,
    index: &HashMap<String, NodeKind>,
    errors: &mut Vec<ValidationError>,
) {
    let refs = collect_all_refs(program);
    for r in &refs {
        let target_id = r.target.as_str();
        // Skip references that are not valid node IDs — e.g. contract predicates
        // like "all_accesses_atomic", "region_valid", "state", "result", "null".
        if !is_node_id(target_id) {
            continue;
        }
        if !index.contains_key(target_id) {
            errors.push(ValidationError {
                error_code: 1002,
                node_id: r.source_id.to_string(),
                violation: "UNDEFINED_REFERENCE".to_string(),
                message: format!(
                    "Node {} referenced by {} is not defined",
                    target_id,
                    r.source_id
                ),
                suggestion: Some(format!(
                    "Check spelling or define {} before use",
                    target_id
                )),
            });
        }
    }
}

/// Collect every NodeRef reference in the program.
fn collect_all_refs(program: &Program) -> Vec<Ref<'_>> {
    let mut refs = Vec::new();

    // T-Nodes
    for t in &program.types {
        collect_type_body_refs(t.id.as_str(), &t.body, &mut refs);
    }

    // R-Nodes
    for r in &program.regions {
        if let Some(parent) = &r.parent {
            refs.push(Ref {
                source_id: r.id.as_str(),
                target: parent,
            });
        }
    }

    // C-Nodes
    for c in &program.computes {
        collect_compute_op_refs(c.id.as_str(), &c.op, &mut refs);
    }

    // E-Nodes
    for e in &program.effects {
        collect_effect_op_refs(e.id.as_str(), &e.op, &mut refs);
    }

    // K-Nodes
    for k in &program.controls {
        collect_control_op_refs(k.id.as_str(), &k.op, &mut refs);
    }

    // V-Nodes
    for v in &program.contracts {
        refs.push(Ref {
            source_id: v.id.as_str(),
            target: &v.target,
        });
        for clause in &v.clauses {
            collect_contract_clause_refs(v.id.as_str(), clause, &mut refs);
        }
    }

    // M-Nodes
    for m in &program.memories {
        collect_memory_op_refs(m.id.as_str(), &m.op, &mut refs);
    }

    // X-Nodes: params/result are TypeRefs, handled via type_ref helper
    for x in &program.externs {
        for p in &x.params {
            collect_type_ref_refs(x.id.as_str(), p, &mut refs);
        }
        collect_type_ref_refs(x.id.as_str(), &x.result, &mut refs);
    }

    // Entry
    refs.push(Ref {
        source_id: program.entry.as_str(),
        target: &program.entry,
    });

    refs
}

fn collect_type_ref_refs<'a>(source: &'a str, tr: &'a TypeRef, out: &mut Vec<Ref<'a>>) {
    if let TypeRef::Id { node } = tr {
        out.push(Ref {
            source_id: source,
            target: node,
        });
    }
}

fn collect_type_body_refs<'a>(source: &'a str, body: &'a TypeBody, out: &mut Vec<Ref<'a>>) {
    match body {
        TypeBody::Array {
            element,
            constraint,
            ..
        } => {
            collect_type_ref_refs(source, element, out);
            if let Some(formula) = constraint {
                collect_formula_refs(source, formula, out);
            }
        }
        TypeBody::Struct { fields, .. } => {
            for f in fields {
                collect_type_ref_refs(source, &f.type_ref, out);
            }
        }
        TypeBody::Variant { cases } => {
            for c in cases {
                collect_type_ref_refs(source, &c.payload, out);
            }
        }
        TypeBody::Fn {
            params, result, ..
        } => {
            for p in params {
                collect_type_ref_refs(source, p, out);
            }
            collect_type_ref_refs(source, result, out);
        }
        TypeBody::Integer { .. }
        | TypeBody::Float { .. }
        | TypeBody::Boolean
        | TypeBody::Unit
        | TypeBody::Opaque { .. } => {}
    }
}

fn collect_compute_op_refs<'a>(source: &'a str, op: &'a ComputeOp, out: &mut Vec<Ref<'a>>) {
    match op {
        ComputeOp::Const {
            type_ref, region, ..
        } => {
            collect_type_ref_refs(source, type_ref, out);
            if let Some(r) = region {
                out.push(Ref {
                    source_id: source,
                    target: r,
                });
            }
        }
        ComputeOp::ConstBytes {
            type_ref, region, ..
        } => {
            collect_type_ref_refs(source, type_ref, out);
            out.push(Ref {
                source_id: source,
                target: region,
            });
        }
        ComputeOp::Arith {
            inputs, type_ref, ..
        } => {
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
        }
        ComputeOp::CallPure {
            inputs, type_ref, ..
        } => {
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
        }
        ComputeOp::Generic {
            inputs,
            type_ref,
            region,
            ..
        } => {
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
            if let Some(r) = region {
                out.push(Ref {
                    source_id: source,
                    target: r,
                });
            }
        }
        ComputeOp::AtomicLoad {
            source: src,
            type_ref,
            ..
        } => {
            out.push(Ref {
                source_id: source,
                target: src,
            });
            collect_type_ref_refs(source, type_ref, out);
        }
        ComputeOp::AtomicStore { target, value, .. } => {
            out.push(Ref {
                source_id: source,
                target,
            });
            out.push(Ref {
                source_id: source,
                target: value,
            });
        }
        ComputeOp::AtomicCas {
            target,
            expected,
            desired,
            success,
            failure,
            ..
        } => {
            out.push(Ref {
                source_id: source,
                target,
            });
            out.push(Ref {
                source_id: source,
                target: expected,
            });
            out.push(Ref {
                source_id: source,
                target: desired,
            });
            out.push(Ref {
                source_id: source,
                target: success,
            });
            out.push(Ref {
                source_id: source,
                target: failure,
            });
        }
    }
}

fn collect_effect_op_refs<'a>(source: &'a str, op: &'a EffectOp, out: &mut Vec<Ref<'a>>) {
    match op {
        EffectOp::Syscall {
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
            if let Some(s) = success {
                out.push(Ref {
                    source_id: source,
                    target: s,
                });
            }
            if let Some(f) = failure {
                out.push(Ref {
                    source_id: source,
                    target: f,
                });
            }
        }
        EffectOp::CallExtern {
            target,
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            out.push(Ref {
                source_id: source,
                target,
            });
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
            out.push(Ref {
                source_id: source,
                target: success,
            });
            out.push(Ref {
                source_id: source,
                target: failure,
            });
        }
        EffectOp::Generic {
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            for i in inputs {
                out.push(Ref {
                    source_id: source,
                    target: i,
                });
            }
            collect_type_ref_refs(source, type_ref, out);
            if let Some(s) = success {
                out.push(Ref {
                    source_id: source,
                    target: s,
                });
            }
            if let Some(f) = failure {
                out.push(Ref {
                    source_id: source,
                    target: f,
                });
            }
        }
    }
}

fn collect_control_op_refs<'a>(source: &'a str, op: &'a ControlOp, out: &mut Vec<Ref<'a>>) {
    match op {
        ControlOp::Seq { steps } => {
            for s in steps {
                out.push(Ref {
                    source_id: source,
                    target: s,
                });
            }
        }
        ControlOp::Branch {
            condition,
            true_branch,
            false_branch,
        } => {
            out.push(Ref {
                source_id: source,
                target: condition,
            });
            out.push(Ref {
                source_id: source,
                target: true_branch,
            });
            out.push(Ref {
                source_id: source,
                target: false_branch,
            });
        }
        ControlOp::Loop {
            condition,
            body,
            state,
            state_type,
        } => {
            out.push(Ref {
                source_id: source,
                target: condition,
            });
            out.push(Ref {
                source_id: source,
                target: body,
            });
            out.push(Ref {
                source_id: source,
                target: state,
            });
            collect_type_ref_refs(source, state_type, out);
        }
        ControlOp::Par { branches, .. } => {
            for b in branches {
                out.push(Ref {
                    source_id: source,
                    target: b,
                });
            }
        }
    }
}

fn collect_contract_clause_refs<'a>(
    source: &'a str,
    clause: &'a ContractClause,
    out: &mut Vec<Ref<'a>>,
) {
    let formula = match clause {
        ContractClause::Pre { formula } => formula,
        ContractClause::Post { formula } => formula,
        ContractClause::Invariant { formula } => formula,
        ContractClause::Assume { formula } => formula,
    };
    collect_formula_refs(source, formula, out);
}

fn collect_formula_refs<'a>(source: &'a str, formula: &'a Formula, out: &mut Vec<Ref<'a>>) {
    match formula {
        Formula::And { left, right } | Formula::Or { left, right } => {
            collect_formula_refs(source, left, out);
            collect_formula_refs(source, right, out);
        }
        Formula::Not { inner } => {
            collect_formula_refs(source, inner, out);
        }
        Formula::Comparison { left, right, .. } => {
            collect_expr_refs(source, left, out);
            collect_expr_refs(source, right, out);
        }
        Formula::Forall {
            range_start,
            range_end,
            body,
            ..
        } => {
            collect_expr_refs(source, range_start, out);
            collect_expr_refs(source, range_end, out);
            collect_formula_refs(source, body, out);
        }
        Formula::BoolLit { .. } => {}
        Formula::FieldAccess { node, .. } => {
            if is_node_id(node.as_str()) {
                out.push(Ref {
                    source_id: source,
                    target: node,
                });
            }
        }
        Formula::PredicateCall { args, .. } => {
            for arg in args {
                collect_formula_refs(source, arg, out);
            }
        }
    }
}

fn collect_expr_refs<'a>(source: &'a str, expr: &'a Expr, out: &mut Vec<Ref<'a>>) {
    match expr {
        Expr::FieldAccess { node, .. } => {
            if is_node_id(node.as_str()) {
                out.push(Ref {
                    source_id: source,
                    target: node,
                });
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_expr_refs(source, left, out);
            collect_expr_refs(source, right, out);
        }
        Expr::PredicateCall { args, .. } => {
            for arg in args {
                collect_expr_refs(source, arg, out);
            }
        }
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::Ident { .. }
        | Expr::Result
        | Expr::State
        | Expr::EmptySet => {}
    }
}

fn collect_memory_op_refs<'a>(source: &'a str, op: &'a MemoryOp, out: &mut Vec<Ref<'a>>) {
    match op {
        MemoryOp::Alloc { type_ref, region } => {
            collect_type_ref_refs(source, type_ref, out);
            out.push(Ref {
                source_id: source,
                target: region,
            });
        }
        MemoryOp::Load {
            source: src,
            index,
            type_ref,
        } => {
            out.push(Ref {
                source_id: source,
                target: src,
            });
            out.push(Ref {
                source_id: source,
                target: index,
            });
            collect_type_ref_refs(source, type_ref, out);
        }
        MemoryOp::Store {
            target,
            index,
            value,
        } => {
            out.push(Ref {
                source_id: source,
                target,
            });
            out.push(Ref {
                source_id: source,
                target: index,
            });
            out.push(Ref {
                source_id: source,
                target: value,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3: DAG check (cycle detection on data-flow edges)
// ---------------------------------------------------------------------------
//
// FTL has two kinds of edges:
//
// 1. **Data edges** — must be acyclic (DAG). These represent value
//    dependencies: C-node inputs, M-node target/value/source pointing to
//    C/M nodes, E-node inputs, E-node target (X-node reference).
//
// 2. **Control-flow edges** — may be cyclic (loops are legitimate).
//    K→K (seq steps, branch targets, loop body, par branches),
//    E→K (success/failure continuations).
//
// The DAG check only considers data edges. Control-flow edges are
// excluded because they form intentional cycles (e.g., loop back-edges).
// ---------------------------------------------------------------------------

/// Returns true if the target ID represents a control-flow node (K: or E:
/// prefix). Edges pointing to these nodes are control-flow edges, not data
/// dependencies, and should be excluded from the DAG acyclicity check.
fn is_control_flow_target(id: &str) -> bool {
    id.starts_with("K:") || id.starts_with("E:")
}

/// Collect data-flow edges for the DAG check.
/// Type references (TypeRef) and region references are excluded.
/// Control-flow edges (K→K, E→K) are excluded — see module doc above.
fn collect_dag_edges(program: &Program) -> HashMap<String, Vec<String>> {
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    // Ensure every node appears in the graph even without outgoing edges.
    for t in &program.types {
        graph.entry(t.id.as_str().to_string()).or_default();
    }
    for r in &program.regions {
        graph.entry(r.id.as_str().to_string()).or_default();
    }
    for c in &program.computes {
        graph.entry(c.id.as_str().to_string()).or_default();
    }
    for e in &program.effects {
        graph.entry(e.id.as_str().to_string()).or_default();
    }
    for k in &program.controls {
        graph.entry(k.id.as_str().to_string()).or_default();
    }
    for v in &program.contracts {
        graph.entry(v.id.as_str().to_string()).or_default();
    }
    for m in &program.memories {
        graph.entry(m.id.as_str().to_string()).or_default();
    }
    for x in &program.externs {
        graph.entry(x.id.as_str().to_string()).or_default();
    }

    // Helper to add an edge from source to target.
    let mut add = |source: &str, target: &str| {
        graph
            .entry(source.to_string())
            .or_default()
            .push(target.to_string());
    };

    // C-Node operational edges (inputs, but NOT type_ref or region)
    for c in &program.computes {
        let s = c.id.as_str();
        match &c.op {
            ComputeOp::Const { .. } => {}
            ComputeOp::ConstBytes { .. } => {}
            ComputeOp::Arith { inputs, .. } => {
                for i in inputs {
                    add(s, i.as_str());
                }
            }
            ComputeOp::CallPure { inputs, .. } => {
                for i in inputs {
                    add(s, i.as_str());
                }
            }
            ComputeOp::Generic { inputs, .. } => {
                for i in inputs {
                    add(s, i.as_str());
                }
            }
            ComputeOp::AtomicLoad { source, .. } => {
                add(s, source.as_str());
            }
            ComputeOp::AtomicStore { target, value, .. } => {
                add(s, target.as_str());
                add(s, value.as_str());
            }
            ComputeOp::AtomicCas {
                target,
                expected,
                desired,
                success,
                failure,
                ..
            } => {
                add(s, target.as_str());
                add(s, expected.as_str());
                add(s, desired.as_str());
                add(s, success.as_str());
                add(s, failure.as_str());
            }
        }
    }

    // E-Node edges: inputs and target are data edges; success/failure are
    // control-flow edges (E→K) and are excluded.
    for e in &program.effects {
        let s = e.id.as_str();
        match &e.op {
            EffectOp::Syscall { inputs, .. } => {
                for i in inputs {
                    if !is_control_flow_target(i.as_str()) {
                        add(s, i.as_str());
                    }
                }
            }
            EffectOp::CallExtern { target, inputs, .. } => {
                // target → X-node (declarative reference, data edge)
                if !is_control_flow_target(target.as_str()) {
                    add(s, target.as_str());
                }
                for i in inputs {
                    if !is_control_flow_target(i.as_str()) {
                        add(s, i.as_str());
                    }
                }
                // success/failure → K-nodes (control-flow, excluded)
            }
            EffectOp::Generic { inputs, .. } => {
                for i in inputs {
                    if !is_control_flow_target(i.as_str()) {
                        add(s, i.as_str());
                    }
                }
                // success/failure → K-nodes (control-flow, excluded)
            }
        }
    }

    // K-Node edges: only edges to C/M nodes are data dependencies.
    // K→K and K→E edges are control-flow and are excluded from the DAG check.
    for k in &program.controls {
        let s = k.id.as_str();
        match &k.op {
            ControlOp::Seq { steps } => {
                // Steps may reference K/E/C/M nodes. Only C/M are data deps.
                for step in steps {
                    if !is_control_flow_target(step.as_str()) {
                        add(s, step.as_str());
                    }
                }
            }
            ControlOp::Branch {
                condition,
                true_branch,
                false_branch,
            } => {
                // condition → C-node (data, keep)
                if !is_control_flow_target(condition.as_str()) {
                    add(s, condition.as_str());
                }
                // true_branch/false_branch → K-nodes (control-flow, skip)
                let _ = (true_branch, false_branch);
            }
            ControlOp::Loop {
                condition,
                body,
                state,
                ..
            } => {
                // condition → C-node (data, keep)
                if !is_control_flow_target(condition.as_str()) {
                    add(s, condition.as_str());
                }
                // state → C-node (data, keep)
                if !is_control_flow_target(state.as_str()) {
                    add(s, state.as_str());
                }
                // body → K-node (control-flow, skip)
                let _ = body;
            }
            ControlOp::Par { branches, .. } => {
                // branches → K-nodes (control-flow, skip)
                let _ = branches;
            }
        }
    }

    // M-Node operational edges
    for m in &program.memories {
        let s = m.id.as_str();
        match &m.op {
            MemoryOp::Alloc { .. } => {
                // region is declarative, not an operational edge
            }
            MemoryOp::Load {
                source, index, ..
            } => {
                add(s, source.as_str());
                add(s, index.as_str());
            }
            MemoryOp::Store {
                target,
                index,
                value,
            } => {
                add(s, target.as_str());
                add(s, index.as_str());
                add(s, value.as_str());
            }
        }
    }

    graph
}

/// DFS-based cycle detection with coloring (White/Gray/Black).
fn check_dag(
    program: &Program,
    _index: &HashMap<String, NodeKind>,
    errors: &mut Vec<ValidationError>,
) {
    let graph = collect_dag_edges(program);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<&str, Color> = HashMap::new();
    for key in graph.keys() {
        color.insert(key.as_str(), Color::White);
    }

    // Track the current DFS path for cycle reporting.
    let mut path: Vec<String> = Vec::new();
    let mut cycles_found: Vec<Vec<String>> = Vec::new();

    fn dfs<'a>(
        node: &'a str,
        graph: &'a HashMap<String, Vec<String>>,
        color: &mut HashMap<&'a str, Color>,
        path: &mut Vec<String>,
        cycles_found: &mut Vec<Vec<String>>,
    ) {
        color.insert(node, Color::Gray);
        path.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                match color.get(neighbor.as_str()) {
                    Some(Color::Gray) => {
                        // Found a cycle. Extract the cycle from the path.
                        let cycle_start = path.iter().position(|n| n == neighbor).unwrap();
                        let cycle: Vec<String> = path[cycle_start..].to_vec();
                        cycles_found.push(cycle);
                    }
                    Some(Color::White) | None => {
                        dfs(neighbor.as_str(), graph, color, path, cycles_found);
                    }
                    Some(Color::Black) => {}
                }
            }
        }

        path.pop();
        color.insert(node, Color::Black);
    }

    // We need to collect keys first to avoid borrow issues.
    let keys: Vec<String> = graph.keys().cloned().collect();
    for key in &keys {
        if color.get(key.as_str()) == Some(&Color::White) {
            dfs(key.as_str(), &graph, &mut color, &mut path, &mut cycles_found);
        }
    }

    // Deduplicate cycles: use the set of nodes as key.
    let mut seen_cycles: HashSet<Vec<String>> = HashSet::new();
    for cycle in cycles_found {
        let mut sorted = cycle.clone();
        sorted.sort();
        if seen_cycles.insert(sorted) {
            let cycle_str = cycle.join(" -> ");
            let first = cycle.first().cloned().unwrap_or_default();
            errors.push(ValidationError {
                error_code: 1003,
                node_id: first,
                violation: "CYCLE_DETECTED".to_string(),
                message: format!("Cycle detected: {} -> ...", cycle_str),
                suggestion: Some(
                    "Break the cycle by removing or redirecting one of the edges".to_string(),
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 4: Entry point validation
// ---------------------------------------------------------------------------

fn check_entry(
    program: &Program,
    index: &HashMap<String, NodeKind>,
    errors: &mut Vec<ValidationError>,
    warnings: &mut Vec<ValidationError>,
) {
    let entry_id = program.entry.as_str();

    // Check if the default entry "K:main" is used (meaning no explicit entry was defined).
    if entry_id == "K:main" {
        // We still check if it exists, but also warn about missing explicit entry.
        warnings.push(ValidationError {
            error_code: 2002,
            node_id: entry_id.to_string(),
            violation: "MISSING_ENTRY".to_string(),
            message: "No explicit entry point defined; using default K:main".to_string(),
            suggestion: Some(
                "Add an explicit ENTRY directive pointing to the main control node".to_string(),
            ),
        });
    }

    match index.get(entry_id) {
        None => {
            errors.push(ValidationError {
                error_code: 1004,
                node_id: entry_id.to_string(),
                violation: "INVALID_ENTRY".to_string(),
                message: format!("Entry point {} does not reference a defined node", entry_id),
                suggestion: Some(format!(
                    "Define a node with ID {} or change the entry point to an existing node",
                    entry_id
                )),
            });
        }
        Some(kind) => {
            if *kind != NodeKind::Control {
                warnings.push(ValidationError {
                    error_code: 2001,
                    node_id: entry_id.to_string(),
                    violation: "ENTRY_NOT_CONTROL".to_string(),
                    message: format!(
                        "Entry point {} is a {:?} node, but should be a K-Node (control)",
                        entry_id, kind
                    ),
                    suggestion: Some(
                        "Change the entry point to reference a K-Node (control flow node)"
                            .to_string(),
                    ),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal valid program with a K:main entry.
    fn minimal_program() -> Program {
        Program {
            types: vec![],
            regions: vec![],
            computes: vec![],
            effects: vec![],
            controls: vec![ControlDef {
                id: NodeRef::new("K:main"),
                op: ControlOp::Seq { steps: vec![] },
            }],
            contracts: vec![],
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new("K:main"),
        }
    }

    #[test]
    fn valid_minimal_program() {
        let p = minimal_program();
        let result = validate(&p);
        assert!(result.valid);
        assert!(result.errors.is_empty());
        // Warning 2002 for default K:main entry
        assert!(result.warnings.iter().any(|w| w.error_code == 2002));
    }

    #[test]
    fn duplicate_id() {
        let mut p = minimal_program();
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:x1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 42 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:x1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 99 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        });

        let result = validate(&p);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == 1001));
    }

    #[test]
    fn undefined_reference() {
        let mut p = minimal_program();
        p.controls = vec![ControlDef {
            id: NodeRef::new("K:main"),
            op: ControlOp::Seq {
                steps: vec![NodeRef::new("C:nonexistent")],
            },
        }];

        let result = validate(&p);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == 1002));
    }

    #[test]
    fn cycle_detected() {
        let mut p = minimal_program();
        // Data-flow cycle: C:a -> C:b -> C:a (via arith inputs)
        p.computes = vec![
            ComputeDef {
                id: NodeRef::new("C:a"),
                op: ComputeOp::Arith {
                    opcode: "add".to_string(),
                    inputs: vec![NodeRef::new("C:b")],
                    type_ref: TypeRef::Builtin { name: "i64".to_string() },
                },
            },
            ComputeDef {
                id: NodeRef::new("C:b"),
                op: ComputeOp::Arith {
                    opcode: "add".to_string(),
                    inputs: vec![NodeRef::new("C:a")],
                    type_ref: TypeRef::Builtin { name: "i64".to_string() },
                },
            },
        ];
        // K:main references C:a (data edge, kept in DAG)
        p.controls = vec![ControlDef {
            id: NodeRef::new("K:main"),
            op: ControlOp::Seq {
                steps: vec![NodeRef::new("C:a")],
            },
        }];

        let result = validate(&p);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == 1003));
    }

    #[test]
    fn k_to_k_cycle_is_control_flow_not_dag_error() {
        let mut p = minimal_program();
        // K→K cycles are control-flow and should NOT trigger a DAG error.
        p.controls = vec![
            ControlDef {
                id: NodeRef::new("K:main"),
                op: ControlOp::Seq {
                    steps: vec![NodeRef::new("K:a")],
                },
            },
            ControlDef {
                id: NodeRef::new("K:a"),
                op: ControlOp::Seq {
                    steps: vec![NodeRef::new("K:b")],
                },
            },
            ControlDef {
                id: NodeRef::new("K:b"),
                op: ControlOp::Seq {
                    steps: vec![NodeRef::new("K:a")],
                },
            },
        ];

        let result = validate(&p);
        assert!(result.valid, "K→K cycles are control-flow, not DAG violations");
        assert!(!result.errors.iter().any(|e| e.error_code == 1003));
    }

    #[test]
    fn invalid_entry() {
        let p = Program {
            types: vec![],
            regions: vec![],
            computes: vec![],
            effects: vec![],
            controls: vec![],
            contracts: vec![],
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new("K:missing"),
        };

        let result = validate(&p);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == 1004));
    }

    #[test]
    fn entry_not_control() {
        let mut p = minimal_program();
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:start"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 0 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        });
        p.entry = NodeRef::new("C:start");

        let result = validate(&p);
        assert!(result.valid); // warning, not error
        assert!(result.warnings.iter().any(|w| w.error_code == 2001));
    }
}
