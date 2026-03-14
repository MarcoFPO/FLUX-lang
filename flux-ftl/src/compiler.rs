// ---------------------------------------------------------------------------
// Phase 5: FTL Compiler — Content-addressed graph with BLAKE3 hashing
// ---------------------------------------------------------------------------
//
// Compiles the FTL AST into a binary, content-addressed graph format.
// Each node gets a BLAKE3 content hash. Same computation = same hash =
// automatic deduplication.
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;

use crate::ast::*;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CompileError {
    /// A NodeRef in the AST does not resolve to any known node.
    UnresolvedRef(String),
    /// Serialization failed for a node.
    SerializationError(String),
    /// I/O error during binary read/write.
    IoError(io::Error),
    /// Invalid binary format.
    InvalidFormat(String),
    /// A value exceeds the maximum representable size in the binary format.
    Overflow(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::UnresolvedRef(id) => write!(f, "unresolved node reference: {}", id),
            CompileError::SerializationError(msg) => write!(f, "serialization error: {}", msg),
            CompileError::IoError(e) => write!(f, "I/O error: {}", e),
            CompileError::InvalidFormat(msg) => write!(f, "invalid binary format: {}", msg),
            CompileError::Overflow(msg) => write!(f, "overflow: {}", msg),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<io::Error> for CompileError {
    fn from(e: io::Error) -> Self {
        CompileError::IoError(e)
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind tag for a compiled node (1-byte discriminator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum NodeKind {
    Type = 0,
    Region = 1,
    Compute = 2,
    Effect = 3,
    Control = 4,
    Contract = 5,
    Memory = 6,
    Extern = 7,
}

impl NodeKind {
    fn from_u8(v: u8) -> Result<Self, CompileError> {
        match v {
            0 => Ok(NodeKind::Type),
            1 => Ok(NodeKind::Region),
            2 => Ok(NodeKind::Compute),
            3 => Ok(NodeKind::Effect),
            4 => Ok(NodeKind::Control),
            5 => Ok(NodeKind::Contract),
            6 => Ok(NodeKind::Memory),
            7 => Ok(NodeKind::Extern),
            _ => Err(CompileError::InvalidFormat(format!("unknown node kind: {}", v))),
        }
    }
}

/// A single compiled node in the content-addressed graph.
#[derive(Debug, Clone)]
pub struct CompiledNode {
    /// BLAKE3 hash of the canonical content (data + sorted refs).
    pub hash: [u8; 32],
    /// The kind discriminator.
    pub kind: NodeKind,
    /// Original FTL identifier, e.g. "C:c1", "E:d1".
    pub original_id: String,
    /// Canonical serialized node data (serde_json bytes).
    pub data: Vec<u8>,
    /// BLAKE3 hashes of all referenced nodes.
    pub refs: Vec<[u8; 32]>,
}

/// The compiled graph: a deduplicated set of content-addressed nodes.
#[derive(Debug, Clone)]
pub struct CompiledGraph {
    /// All unique compiled nodes.
    pub nodes: Vec<CompiledNode>,
    /// Hash of the entry node.
    pub entry_hash: [u8; 32],
    /// Summary metadata.
    pub metadata: GraphMetadata,
}

/// Summary metadata for JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct GraphMetadata {
    pub total_nodes: usize,
    pub unique_nodes: usize,
    pub entry_id: String,
}

/// Metadata emitted in the main pipeline's JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct CompileMetadata {
    pub entry_hash: String,
    pub total_nodes: usize,
    pub unique_nodes: usize,
}

impl From<&CompiledGraph> for CompileMetadata {
    fn from(g: &CompiledGraph) -> Self {
        CompileMetadata {
            entry_hash: hex_encode(&g.entry_hash),
            total_nodes: g.metadata.total_nodes,
            unique_nodes: g.metadata.unique_nodes,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Compute the content hash: BLAKE3(data || sorted_ref_hashes).
fn hash_node(node_data: &[u8], refs: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(node_data);

    // Sort references for determinism.
    let mut sorted_refs: Vec<[u8; 32]> = refs.to_vec();
    sorted_refs.sort();
    for r in &sorted_refs {
        hasher.update(r);
    }

    *hasher.finalize().as_bytes()
}

/// Serialize a value to canonical JSON bytes for hashing.
fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CompileError> {
    serde_json::to_vec(value).map_err(|e| CompileError::SerializationError(e.to_string()))
}

// ---------------------------------------------------------------------------
// Reference extraction — collect all NodeRef strings from each node type
// ---------------------------------------------------------------------------

fn refs_from_type_ref(tr: &TypeRef) -> Vec<String> {
    match tr {
        TypeRef::Id { node } => vec![node.0.clone()],
        TypeRef::Builtin { .. } => vec![],
    }
}

fn refs_from_formula(f: &Formula) -> Vec<String> {
    match f {
        Formula::And { left, right } | Formula::Or { left, right } => {
            let mut v = refs_from_formula(left);
            v.extend(refs_from_formula(right));
            v
        }
        Formula::Not { inner } => refs_from_formula(inner),
        Formula::Comparison { left, right, .. } => {
            let mut v = refs_from_expr(left);
            v.extend(refs_from_expr(right));
            v
        }
        Formula::Forall { range_start, range_end, body, .. } => {
            let mut v = refs_from_expr(range_start);
            v.extend(refs_from_expr(range_end));
            v.extend(refs_from_formula(body));
            v
        }
        Formula::BoolLit { .. } => vec![],
        Formula::FieldAccess { node, .. } => vec![node.0.clone()],
        Formula::PredicateCall { args, .. } => {
            args.iter().flat_map(refs_from_formula).collect()
        }
    }
}

fn refs_from_expr(e: &Expr) -> Vec<String> {
    match e {
        Expr::IntLit { .. } | Expr::FloatLit { .. } | Expr::Ident { .. }
        | Expr::Result | Expr::State | Expr::EmptySet => vec![],
        Expr::FieldAccess { node, .. } => vec![node.0.clone()],
        Expr::BinOp { left, right, .. } => {
            let mut v = refs_from_expr(left);
            v.extend(refs_from_expr(right));
            v
        }
        Expr::PredicateCall { args, .. } => {
            args.iter().flat_map(refs_from_expr).collect()
        }
    }
}

fn refs_for_type(t: &TypeDef) -> Vec<String> {
    match &t.body {
        TypeBody::Struct { fields, .. } => {
            fields.iter().flat_map(|f| refs_from_type_ref(&f.type_ref)).collect()
        }
        TypeBody::Array { element, constraint, .. } => {
            let mut v = refs_from_type_ref(element);
            if let Some(formula) = constraint {
                v.extend(refs_from_formula(formula));
            }
            v
        }
        TypeBody::Variant { cases } => {
            cases.iter().flat_map(|c| refs_from_type_ref(&c.payload)).collect()
        }
        TypeBody::Fn { params, result, .. } => {
            let mut v: Vec<String> = params.iter().flat_map(refs_from_type_ref).collect();
            v.extend(refs_from_type_ref(result));
            v
        }
        TypeBody::Integer { .. } | TypeBody::Float { .. }
        | TypeBody::Boolean | TypeBody::Unit | TypeBody::Opaque { .. } => vec![],
    }
}

fn refs_for_region(r: &RegionDef) -> Vec<String> {
    r.parent.as_ref().map_or_else(Vec::new, |p| vec![p.0.clone()])
}

fn refs_for_compute(c: &ComputeDef) -> Vec<String> {
    match &c.op {
        ComputeOp::Const { type_ref, region, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            if let Some(r) = region {
                v.push(r.0.clone());
            }
            v
        }
        ComputeOp::ConstBytes { type_ref, region, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(region.0.clone());
            v
        }
        ComputeOp::Arith { inputs, type_ref, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            v
        }
        ComputeOp::CallPure { inputs, type_ref, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            v
        }
        ComputeOp::Generic { inputs, type_ref, region, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(r) = region {
                v.push(r.0.clone());
            }
            v
        }
        ComputeOp::AtomicLoad { source, type_ref, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(source.0.clone());
            v
        }
        ComputeOp::AtomicStore { target, value, .. } => {
            vec![target.0.clone(), value.0.clone()]
        }
        ComputeOp::AtomicCas { target, expected, desired, success, failure, .. } => {
            vec![
                target.0.clone(), expected.0.clone(), desired.0.clone(),
                success.0.clone(), failure.0.clone(),
            ]
        }
    }
}

fn refs_for_effect(e: &EffectDef) -> Vec<String> {
    match &e.op {
        EffectOp::Syscall { inputs, type_ref, success, failure, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(s) = success { v.push(s.0.clone()); }
            if let Some(f) = failure { v.push(f.0.clone()); }
            v
        }
        EffectOp::CallExtern { target, inputs, type_ref, success, failure, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(target.0.clone());
            v.extend(inputs.iter().map(|i| i.0.clone()));
            v.push(success.0.clone());
            v.push(failure.0.clone());
            v
        }
        EffectOp::Generic { inputs, type_ref, success, failure, .. } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(s) = success { v.push(s.0.clone()); }
            if let Some(f) = failure { v.push(f.0.clone()); }
            v
        }
    }
}

fn refs_for_control(k: &ControlDef) -> Vec<String> {
    match &k.op {
        ControlOp::Seq { steps } => steps.iter().map(|s| s.0.clone()).collect(),
        ControlOp::Branch { condition, true_branch, false_branch } => {
            vec![condition.0.clone(), true_branch.0.clone(), false_branch.0.clone()]
        }
        ControlOp::Loop { condition, body, state, state_type, .. } => {
            let mut v = vec![condition.0.clone(), body.0.clone(), state.0.clone()];
            v.extend(refs_from_type_ref(state_type));
            v
        }
        ControlOp::Par { branches, .. } => {
            branches.iter().map(|b| b.0.clone()).collect()
        }
    }
}

fn refs_for_contract(c: &ContractDef) -> Vec<String> {
    let mut v = vec![c.target.0.clone()];
    for clause in &c.clauses {
        let formula = match clause {
            ContractClause::Pre { formula } => formula,
            ContractClause::Post { formula } => formula,
            ContractClause::Invariant { formula } => formula,
            ContractClause::Assume { formula } => formula,
        };
        v.extend(refs_from_formula(formula));
    }
    v
}

fn refs_for_memory(m: &MemoryDef) -> Vec<String> {
    match &m.op {
        MemoryOp::Alloc { type_ref, region } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(region.0.clone());
            v
        }
        MemoryOp::Load { source, index, type_ref } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(source.0.clone());
            v.push(index.0.clone());
            v
        }
        MemoryOp::Store { target, index, value } => {
            vec![target.0.clone(), index.0.clone(), value.0.clone()]
        }
    }
}

fn refs_for_extern(x: &ExternDef) -> Vec<String> {
    let mut v: Vec<String> = x.params.iter().flat_map(refs_from_type_ref).collect();
    v.extend(refs_from_type_ref(&x.result));
    v
}

// ---------------------------------------------------------------------------
// Reachability analysis — dead node elimination
// ---------------------------------------------------------------------------

fn collect_reachable(program: &Program) -> HashSet<String> {
    // Build adjacency: id -> list of referenced ids
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    for t in &program.types {
        adj.insert(t.id.0.clone(), refs_for_type(t));
    }
    for r in &program.regions {
        adj.insert(r.id.0.clone(), refs_for_region(r));
    }
    for c in &program.computes {
        adj.insert(c.id.0.clone(), refs_for_compute(c));
    }
    for e in &program.effects {
        adj.insert(e.id.0.clone(), refs_for_effect(e));
    }
    for k in &program.controls {
        adj.insert(k.id.0.clone(), refs_for_control(k));
    }
    for v in &program.contracts {
        adj.insert(v.id.0.clone(), refs_for_contract(v));
    }
    for m in &program.memories {
        adj.insert(m.id.0.clone(), refs_for_memory(m));
    }
    for x in &program.externs {
        adj.insert(x.id.0.clone(), refs_for_extern(x));
    }

    // BFS from entry
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Start from the entry node
    queue.push_back(program.entry.0.clone());

    // Also start from all contract nodes (contracts are always reachable
    // since they are verification obligations, not data-flow nodes).
    for v in &program.contracts {
        queue.push_back(v.id.0.clone());
    }

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(neighbors) = adj.get(&id) {
            for n in neighbors {
                if !visited.contains(n) {
                    queue.push_back(n.clone());
                }
            }
        }
    }

    visited
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

/// Compile an FTL AST into a content-addressed graph.
pub fn compile(program: &Program) -> Result<CompiledGraph, CompileError> {
    let reachable = collect_reachable(program);

    // Phase 1: Compute canonical bytes and collect raw refs for every node.
    // We store (original_id, kind, data_bytes, ref_id_strings).
    let mut raw_nodes: Vec<(String, NodeKind, Vec<u8>, Vec<String>)> = Vec::new();

    for t in &program.types {
        if !reachable.contains(&t.id.0) { continue; }
        let data = canonical_bytes(&t.body)?;
        raw_nodes.push((t.id.0.clone(), NodeKind::Type, data, refs_for_type(t)));
    }
    for r in &program.regions {
        if !reachable.contains(&r.id.0) { continue; }
        // Serialize just lifetime + parent presence for canonical form.
        let data = canonical_bytes(&(&r.lifetime, &r.parent))?;
        raw_nodes.push((r.id.0.clone(), NodeKind::Region, data, refs_for_region(r)));
    }
    for c in &program.computes {
        if !reachable.contains(&c.id.0) { continue; }
        let data = canonical_bytes(&c.op)?;
        raw_nodes.push((c.id.0.clone(), NodeKind::Compute, data, refs_for_compute(c)));
    }
    for e in &program.effects {
        if !reachable.contains(&e.id.0) { continue; }
        let data = canonical_bytes(&e.op)?;
        raw_nodes.push((e.id.0.clone(), NodeKind::Effect, data, refs_for_effect(e)));
    }
    for k in &program.controls {
        if !reachable.contains(&k.id.0) { continue; }
        let data = canonical_bytes(&k.op)?;
        raw_nodes.push((k.id.0.clone(), NodeKind::Control, data, refs_for_control(k)));
    }
    for v in &program.contracts {
        if !reachable.contains(&v.id.0) { continue; }
        let data = canonical_bytes(&(&v.clauses, &v.trust))?;
        raw_nodes.push((v.id.0.clone(), NodeKind::Contract, data, refs_for_contract(v)));
    }
    for m in &program.memories {
        if !reachable.contains(&m.id.0) { continue; }
        let data = canonical_bytes(&m.op)?;
        raw_nodes.push((m.id.0.clone(), NodeKind::Memory, data, refs_for_memory(m)));
    }
    for x in &program.externs {
        if !reachable.contains(&x.id.0) { continue; }
        let data = canonical_bytes(&(&x.name, &x.abi, &x.params, &x.result, &x.effects))?;
        raw_nodes.push((x.id.0.clone(), NodeKind::Extern, data, refs_for_extern(x)));
    }

    let total_nodes = raw_nodes.len();

    // Phase 2: Compute hashes bottom-up.
    // First pass: hash leaf nodes (those with no refs or only refs to builtins).
    // We iterate until all nodes are hashed.
    let mut hash_map: HashMap<String, [u8; 32]> = HashMap::new();

    // Iterative resolution: keep hashing nodes whose refs are all resolved.
    let mut remaining: Vec<(String, NodeKind, Vec<u8>, Vec<String>)> = raw_nodes;
    let mut max_iterations = remaining.len() + 1;

    while !remaining.is_empty() && max_iterations > 0 {
        max_iterations -= 1;
        let mut still_remaining = Vec::new();

        for (id, kind, data, ref_ids) in remaining {
            // Check if all refs are resolved.
            let all_resolved = ref_ids.iter().all(|r| hash_map.contains_key(r));

            if all_resolved {
                let ref_hashes: Vec<[u8; 32]> = ref_ids
                    .iter()
                    .filter_map(|r| hash_map.get(r).copied())
                    .collect();
                let h = hash_node(&data, &ref_hashes);
                hash_map.insert(id, h);
            } else {
                still_remaining.push((id, kind, data, ref_ids));
            }
        }

        remaining = still_remaining;
    }

    // If there are still remaining nodes, there are cycles or unresolved refs.
    // Handle cycles by hashing with a zero-hash placeholder for unresolved refs.
    for (id, _kind, data, ref_ids) in &remaining {
        let ref_hashes: Vec<[u8; 32]> = ref_ids
            .iter()
            .map(|r| hash_map.get(r).copied().unwrap_or([0u8; 32]))
            .collect();
        let h = hash_node(data, &ref_hashes);
        hash_map.insert(id.clone(), h);
    }

    // Phase 3: Build compiled nodes, deduplicate by hash.
    // Re-collect all nodes (we consumed them above, so rebuild from program).
    let mut all_compiled: Vec<CompiledNode> = Vec::new();
    let mut seen_hashes: HashSet<[u8; 32]> = HashSet::new();

    // Helper closure to build and dedup a node.
    let mut add_node = |id: &str, kind: NodeKind, data: Vec<u8>, ref_ids: &[String]| {
        if let Some(&h) = hash_map.get(id)
            && seen_hashes.insert(h)
        {
            let ref_hashes: Vec<[u8; 32]> = ref_ids
                .iter()
                .filter_map(|r| hash_map.get(r.as_str()).copied())
                .collect();
            all_compiled.push(CompiledNode {
                hash: h,
                kind,
                original_id: id.to_string(),
                data,
                refs: ref_hashes,
            });
        }
    };

    for t in &program.types {
        if !reachable.contains(&t.id.0) { continue; }
        let data = canonical_bytes(&t.body)?;
        let r = refs_for_type(t);
        add_node(&t.id.0, NodeKind::Type, data, &r);
    }
    for r in &program.regions {
        if !reachable.contains(&r.id.0) { continue; }
        let data = canonical_bytes(&(&r.lifetime, &r.parent))?;
        let refs = refs_for_region(r);
        add_node(&r.id.0, NodeKind::Region, data, &refs);
    }
    for c in &program.computes {
        if !reachable.contains(&c.id.0) { continue; }
        let data = canonical_bytes(&c.op)?;
        let refs = refs_for_compute(c);
        add_node(&c.id.0, NodeKind::Compute, data, &refs);
    }
    for e in &program.effects {
        if !reachable.contains(&e.id.0) { continue; }
        let data = canonical_bytes(&e.op)?;
        let refs = refs_for_effect(e);
        add_node(&e.id.0, NodeKind::Effect, data, &refs);
    }
    for k in &program.controls {
        if !reachable.contains(&k.id.0) { continue; }
        let data = canonical_bytes(&k.op)?;
        let refs = refs_for_control(k);
        add_node(&k.id.0, NodeKind::Control, data, &refs);
    }
    for v in &program.contracts {
        if !reachable.contains(&v.id.0) { continue; }
        let data = canonical_bytes(&(&v.clauses, &v.trust))?;
        let refs = refs_for_contract(v);
        add_node(&v.id.0, NodeKind::Contract, data, &refs);
    }
    for m in &program.memories {
        if !reachable.contains(&m.id.0) { continue; }
        let data = canonical_bytes(&m.op)?;
        let refs = refs_for_memory(m);
        add_node(&m.id.0, NodeKind::Memory, data, &refs);
    }
    for x in &program.externs {
        if !reachable.contains(&x.id.0) { continue; }
        let data = canonical_bytes(&(&x.name, &x.abi, &x.params, &x.result, &x.effects))?;
        let refs = refs_for_extern(x);
        add_node(&x.id.0, NodeKind::Extern, data, &refs);
    }

    let entry_hash = hash_map
        .get(&program.entry.0)
        .copied()
        .ok_or_else(|| CompileError::UnresolvedRef(program.entry.0.clone()))?;

    let unique_nodes = all_compiled.len();

    Ok(CompiledGraph {
        nodes: all_compiled,
        entry_hash,
        metadata: GraphMetadata {
            total_nodes,
            unique_nodes,
            entry_id: program.entry.0.clone(),
        },
    })
}

// ---------------------------------------------------------------------------
// Binary format: read / write
// ---------------------------------------------------------------------------
//
// Format:
//   Magic:       b"FLUX"   (4 bytes)
//   Version:     2u32 LE   (4 bytes)
//   Entry hash:  [u8; 32]
//   Node count:  u32 LE    (unique nodes)
//   Total nodes: u32 LE    (before deduplication)
//   Per node:
//     hash:      [u8; 32]
//     kind:      u8
//     id_len:    u16 LE
//     id:        [u8; id_len]
//     data_len:  u32 LE
//     data:      [u8; data_len]
//     ref_count: u16 LE
//     refs:      [u8; 32] * ref_count
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"FLUX";
const VERSION: u32 = 2;

/// Write a compiled graph to a binary `.flux.bin` file.
pub fn write_binary(graph: &CompiledGraph, path: &std::path::Path) -> Result<(), CompileError> {
    let mut file = std::fs::File::create(path)?;

    // Magic
    file.write_all(MAGIC)?;
    // Version
    file.write_all(&VERSION.to_le_bytes())?;
    // Entry hash
    file.write_all(&graph.entry_hash)?;
    // Node count
    let count = u32::try_from(graph.nodes.len())
        .map_err(|_| CompileError::Overflow(format!("node count {} exceeds u32::MAX", graph.nodes.len())))?;
    file.write_all(&count.to_le_bytes())?;
    // Total nodes (before deduplication)
    let total_nodes = u32::try_from(graph.metadata.total_nodes)
        .map_err(|_| CompileError::Overflow(format!("total_nodes {} exceeds u32::MAX", graph.metadata.total_nodes)))?;
    file.write_all(&total_nodes.to_le_bytes())?;

    for node in &graph.nodes {
        // Hash
        file.write_all(&node.hash)?;
        // Kind
        file.write_all(&[node.kind as u8])?;
        // ID
        let id_bytes = node.original_id.as_bytes();
        let id_len = u16::try_from(id_bytes.len())
            .map_err(|_| CompileError::Overflow(format!("node id length {} exceeds u16::MAX", id_bytes.len())))?;
        file.write_all(&id_len.to_le_bytes())?;
        file.write_all(id_bytes)?;
        // Data
        let data_len = u32::try_from(node.data.len())
            .map_err(|_| CompileError::Overflow(format!("node data length {} exceeds u32::MAX", node.data.len())))?;
        file.write_all(&data_len.to_le_bytes())?;
        file.write_all(&node.data)?;
        // Refs
        let ref_count = u16::try_from(node.refs.len())
            .map_err(|_| CompileError::Overflow(format!("ref count {} exceeds u16::MAX", node.refs.len())))?;
        file.write_all(&ref_count.to_le_bytes())?;
        for r in &node.refs {
            file.write_all(r)?;
        }
    }

    file.flush()?;
    Ok(())
}

/// Read a compiled graph from a binary `.flux.bin` file.
pub fn read_binary(path: &std::path::Path) -> Result<CompiledGraph, CompileError> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let mut pos = 0;

    // Helper to read exact bytes
    let read_bytes = |pos: &mut usize, n: usize| -> Result<&[u8], CompileError> {
        if *pos + n > buf.len() {
            return Err(CompileError::InvalidFormat("unexpected end of file".into()));
        }
        let slice = &buf[*pos..*pos + n];
        *pos += n;
        Ok(slice)
    };

    // Magic
    let magic = read_bytes(&mut pos, 4)?;
    if magic != MAGIC {
        return Err(CompileError::InvalidFormat("invalid magic bytes".into()));
    }

    // Version
    let ver_bytes = read_bytes(&mut pos, 4)?;
    let version = u32::from_le_bytes([ver_bytes[0], ver_bytes[1], ver_bytes[2], ver_bytes[3]]);
    if version != VERSION {
        return Err(CompileError::InvalidFormat(format!(
            "unsupported version: {}", version
        )));
    }

    // Entry hash
    let entry_slice = read_bytes(&mut pos, 32)?;
    let mut entry_hash = [0u8; 32];
    entry_hash.copy_from_slice(entry_slice);

    // Node count
    let count_bytes = read_bytes(&mut pos, 4)?;
    let count = u32::from_le_bytes([
        count_bytes[0], count_bytes[1], count_bytes[2], count_bytes[3],
    ]) as usize;

    // Total nodes (before deduplication)
    let total_bytes = read_bytes(&mut pos, 4)?;
    let total_nodes = u32::from_le_bytes([
        total_bytes[0], total_bytes[1], total_bytes[2], total_bytes[3],
    ]) as usize;

    let mut nodes = Vec::with_capacity(count);
    let mut entry_id = String::new();

    for _ in 0..count {
        // Hash
        let h_slice = read_bytes(&mut pos, 32)?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(h_slice);

        // Kind
        let kind_byte = read_bytes(&mut pos, 1)?[0];
        let kind = NodeKind::from_u8(kind_byte)?;

        // ID
        let id_len_bytes = read_bytes(&mut pos, 2)?;
        let id_len = u16::from_le_bytes([id_len_bytes[0], id_len_bytes[1]]) as usize;
        let id_bytes = read_bytes(&mut pos, id_len)?;
        let original_id = String::from_utf8(id_bytes.to_vec())
            .map_err(|e| CompileError::InvalidFormat(format!("invalid UTF-8 in node id: {}", e)))?;

        if hash == entry_hash && entry_id.is_empty() {
            entry_id.clone_from(&original_id);
        }

        // Data
        let data_len_bytes = read_bytes(&mut pos, 4)?;
        let data_len = u32::from_le_bytes([
            data_len_bytes[0], data_len_bytes[1], data_len_bytes[2], data_len_bytes[3],
        ]) as usize;
        let data = read_bytes(&mut pos, data_len)?.to_vec();

        // Refs
        let ref_count_bytes = read_bytes(&mut pos, 2)?;
        let ref_count = u16::from_le_bytes([ref_count_bytes[0], ref_count_bytes[1]]) as usize;
        let mut refs = Vec::with_capacity(ref_count);
        for _ in 0..ref_count {
            let r_slice = read_bytes(&mut pos, 32)?;
            let mut r = [0u8; 32];
            r.copy_from_slice(r_slice);
            refs.push(r);
        }

        nodes.push(CompiledNode {
            hash,
            kind,
            original_id,
            data,
            refs,
        });
    }

    let unique_nodes = nodes.len();

    Ok(CompiledGraph {
        entry_hash,
        metadata: GraphMetadata {
            total_nodes,
            unique_nodes,
            entry_id,
        },
        nodes,
    })
}
