// ---------------------------------------------------------------------------
// optimizer.rs — Graph-level optimizations and LLVM pass manager integration
// ---------------------------------------------------------------------------
//
// Phase 11: Superoptimizer — 3-tier optimization pipeline:
//   Tier 1: Graph-level optimizations (pre-LLVM)
//     - Dead node elimination
//     - Constant folding for Arith C-Nodes
//     - Identity removal (add 0, mul 1, etc.)
//   Tier 2: LLVM optimization passes (-O1 through -O3)
//   Tier 3: (Future) MLIR + STOKE stochastic superoptimization
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet, VecDeque};

use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::values::FunctionValue;

use crate::ast::*;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Controls which optimizations are applied.
#[derive(Debug, Clone)]
pub struct OptimizationConfig {
    /// LLVM optimization level (0-3).
    pub llvm_opt_level: u8,
    /// Enable graph-level optimizations (dead node elimination, constant
    /// folding, identity removal).
    pub enable_graph_opts: bool,
    /// Remove nodes not reachable from the entry point.
    pub strip_dead_nodes: bool,
    /// Fold arithmetic on constant inputs at compile time.
    pub fold_constants: bool,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            llvm_opt_level: 2,
            enable_graph_opts: true,
            strip_dead_nodes: true,
            fold_constants: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// The result of graph-level optimization.
pub struct OptimizationResult {
    pub optimized_program: Program,
    pub stats: OptStats,
}

/// Statistics collected during optimization.
#[derive(Debug, Clone, Default)]
pub struct OptStats {
    pub nodes_before: usize,
    pub nodes_after: usize,
    pub constants_folded: usize,
    pub dead_nodes_removed: usize,
    pub identities_removed: usize,
}

// ---------------------------------------------------------------------------
// Graph-level optimization entry point
// ---------------------------------------------------------------------------

/// Apply graph-level optimizations to an FTL program.
///
/// This runs before LLVM codegen and operates on the AST directly:
///   1. Constant folding — evaluate `Arith` C-Nodes with all-const inputs
///   2. Identity removal — remove no-op operations (add 0, mul 1, etc.)
///   3. Dead node elimination — strip nodes unreachable from entry
pub fn optimize_graph(program: &Program, config: &OptimizationConfig) -> OptimizationResult {
    let mut prog = program.clone();
    let mut stats = OptStats::default();

    let total_node_count = |p: &Program| -> usize {
        p.types.len()
            + p.regions.len()
            + p.computes.len()
            + p.effects.len()
            + p.controls.len()
            + p.contracts.len()
            + p.memories.len()
            + p.externs.len()
    };

    stats.nodes_before = total_node_count(&prog);

    if !config.enable_graph_opts {
        stats.nodes_after = stats.nodes_before;
        return OptimizationResult {
            optimized_program: prog,
            stats,
        };
    }

    // Pass 1: Constant folding
    if config.fold_constants {
        let folded = fold_constants(&prog);
        stats.constants_folded = folded.constants_folded;
        prog = folded.program;
    }

    // Pass 2: Identity removal (piggy-backs on constant folding results)
    let identity_result = remove_identities(&prog);
    stats.identities_removed = identity_result.identities_removed;
    prog = identity_result.program;

    // Pass 3: Dead node elimination
    if config.strip_dead_nodes {
        let before = total_node_count(&prog);
        prog = eliminate_dead_nodes(&prog);
        let after = total_node_count(&prog);
        stats.dead_nodes_removed = before.saturating_sub(after);
    }

    stats.nodes_after = total_node_count(&prog);

    OptimizationResult {
        optimized_program: prog,
        stats,
    }
}

// ---------------------------------------------------------------------------
// Constant folding
// ---------------------------------------------------------------------------

struct FoldResult {
    program: Program,
    constants_folded: usize,
}

/// Build a map from C-node id to its literal value (if it is a Const node).
fn build_const_map(computes: &[ComputeDef]) -> HashMap<String, &Literal> {
    let mut map = HashMap::new();
    for c in computes {
        if let ComputeOp::Const { value, .. } = &c.op {
            map.insert(c.id.0.clone(), value);
        }
    }
    map
}

/// Try to evaluate a binary integer operation at compile time.
fn eval_int_op(opcode: &str, a: i64, b: i64) -> Option<i64> {
    match opcode {
        "add" => Some(a.wrapping_add(b)),
        "sub" => Some(a.wrapping_sub(b)),
        "mul" => Some(a.wrapping_mul(b)),
        "div" if b != 0 => Some(a.wrapping_div(b)),
        "mod" if b != 0 => Some(a.wrapping_rem(b)),
        _ => None,
    }
}

/// Try to evaluate a binary float operation at compile time.
fn eval_float_op(opcode: &str, a: f64, b: f64) -> Option<f64> {
    match opcode {
        "add" => Some(a + b),
        "sub" => Some(a - b),
        "mul" => Some(a * b),
        "div" if b != 0.0 => Some(a / b),
        _ => None,
    }
}

fn fold_constants(program: &Program) -> FoldResult {
    let const_map = build_const_map(&program.computes);
    let mut folded_count: usize = 0;

    let new_computes: Vec<ComputeDef> = program
        .computes
        .iter()
        .map(|c| {
            if let ComputeOp::Arith {
                opcode,
                inputs,
                type_ref,
            } = &c.op
                && inputs.len() == 2
            {
                let lhs = const_map.get(&inputs[0].0);
                let rhs = const_map.get(&inputs[1].0);

                if let (Some(lhs_lit), Some(rhs_lit)) = (lhs, rhs)
                    && let Some(folded) = try_fold(opcode, lhs_lit, rhs_lit)
                {
                    folded_count += 1;
                    return ComputeDef {
                        id: c.id.clone(),
                        op: ComputeOp::Const {
                            value: folded,
                            type_ref: type_ref.clone(),
                            region: None,
                        },
                    };
                }
            }
            c.clone()
        })
        .collect();

    FoldResult {
        program: Program {
            computes: new_computes,
            types: program.types.clone(),
            regions: program.regions.clone(),
            effects: program.effects.clone(),
            controls: program.controls.clone(),
            contracts: program.contracts.clone(),
            memories: program.memories.clone(),
            externs: program.externs.clone(),
            entry: program.entry.clone(),
        },
        constants_folded: folded_count,
    }
}

/// Attempt to fold two constant literals with the given opcode.
fn try_fold(opcode: &str, lhs: &Literal, rhs: &Literal) -> Option<Literal> {
    match (lhs, rhs) {
        (Literal::Integer { value: a }, Literal::Integer { value: b }) => {
            eval_int_op(opcode, *a, *b).map(|v| Literal::Integer { value: v })
        }
        (Literal::Float { value: a }, Literal::Float { value: b }) => {
            eval_float_op(opcode, *a, *b).map(|v| Literal::Float { value: v })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Identity removal
// ---------------------------------------------------------------------------

struct IdentityResult {
    program: Program,
    identities_removed: usize,
}

/// Detect and simplify identity operations:
///   - add(x, 0) or add(0, x) -> x
///   - sub(x, 0) -> x
///   - mul(x, 1) or mul(1, x) -> x
///   - mul(x, 0) or mul(0, x) -> const(0)
///   - div(x, 1) -> x
///
/// When an identity is found, the Arith node is replaced with a Const or
/// the node is rewritten to reference the non-identity input directly.
/// Because the AST uses IDs for all references, a full rewrite of the
/// Arith node into the passthrough value requires either:
///   a) replacing the Arith with a clone of the passthrough Const, or
///   b) maintaining a substitution map.
///
/// We use approach (a): if one operand is a known identity constant and
/// the other is also a Const, we replace the node with that Const value.
/// If the other operand is not a Const, we leave the node in place — LLVM
/// will handle this trivially.
fn remove_identities(program: &Program) -> IdentityResult {
    let const_map = build_const_map(&program.computes);
    let mut removed: usize = 0;

    let new_computes: Vec<ComputeDef> = program
        .computes
        .iter()
        .map(|c| {
            if let ComputeOp::Arith {
                opcode,
                inputs,
                type_ref,
            } = &c.op
                && inputs.len() == 2
                && let Some(replacement) =
                    try_remove_identity(opcode, &inputs[0], &inputs[1], type_ref, &const_map)
            {
                removed += 1;
                return ComputeDef {
                    id: c.id.clone(),
                    op: replacement,
                };
            }
            c.clone()
        })
        .collect();

    IdentityResult {
        program: Program {
            computes: new_computes,
            types: program.types.clone(),
            regions: program.regions.clone(),
            effects: program.effects.clone(),
            controls: program.controls.clone(),
            contracts: program.contracts.clone(),
            memories: program.memories.clone(),
            externs: program.externs.clone(),
            entry: program.entry.clone(),
        },
        identities_removed: removed,
    }
}

/// Check whether an Arith node is an identity operation. If so, return
/// the replacement ComputeOp.
fn try_remove_identity(
    opcode: &str,
    lhs_ref: &NodeRef,
    rhs_ref: &NodeRef,
    type_ref: &TypeRef,
    const_map: &HashMap<String, &Literal>,
) -> Option<ComputeOp> {
    let lhs = const_map.get(&lhs_ref.0);
    let rhs = const_map.get(&rhs_ref.0);

    // Helper to build a replacement Const node from a literal.
    let make_const = |lit: &Literal| -> ComputeOp {
        ComputeOp::Const {
            value: lit.clone(),
            type_ref: type_ref.clone(),
            region: None,
        }
    };

    match opcode {
        "add" => {
            // add(x, 0) -> x   or   add(0, x) -> x
            if is_zero_int(rhs) && let Some(lhs_val) = lhs {
                return Some(make_const(lhs_val));
            }
            if is_zero_int(lhs) && let Some(rhs_val) = rhs {
                return Some(make_const(rhs_val));
            }
            None
        }
        "sub" => {
            // sub(x, 0) -> x
            if is_zero_int(rhs) && let Some(lhs_val) = lhs {
                return Some(make_const(lhs_val));
            }
            None
        }
        "mul" => {
            // mul(x, 0) -> 0   or   mul(0, x) -> 0
            if is_zero_int(rhs) || is_zero_int(lhs) {
                return Some(ComputeOp::Const {
                    value: Literal::Integer { value: 0 },
                    type_ref: type_ref.clone(),
                    region: None,
                });
            }
            // mul(x, 1) -> x   or   mul(1, x) -> x
            if is_one_int(rhs) && let Some(lhs_val) = lhs {
                return Some(make_const(lhs_val));
            }
            if is_one_int(lhs) && let Some(rhs_val) = rhs {
                return Some(make_const(rhs_val));
            }
            None
        }
        "div" => {
            // div(x, 1) -> x
            if is_one_int(rhs) && let Some(lhs_val) = lhs {
                return Some(make_const(lhs_val));
            }
            None
        }
        _ => None,
    }
}

fn is_zero_int(lit: Option<&&Literal>) -> bool {
    matches!(lit, Some(Literal::Integer { value: 0 }))
}

fn is_one_int(lit: Option<&&Literal>) -> bool {
    matches!(lit, Some(Literal::Integer { value: 1 }))
}

// ---------------------------------------------------------------------------
// Dead node elimination
// ---------------------------------------------------------------------------

/// Remove nodes not reachable from the program entry point.
/// Contracts are always kept (they are verification obligations).
fn eliminate_dead_nodes(program: &Program) -> Program {
    let reachable = collect_reachable(program);

    Program {
        types: program
            .types
            .iter()
            .filter(|t| reachable.contains(&t.id.0))
            .cloned()
            .collect(),
        regions: program
            .regions
            .iter()
            .filter(|r| reachable.contains(&r.id.0))
            .cloned()
            .collect(),
        computes: program
            .computes
            .iter()
            .filter(|c| reachable.contains(&c.id.0))
            .cloned()
            .collect(),
        effects: program
            .effects
            .iter()
            .filter(|e| reachable.contains(&e.id.0))
            .cloned()
            .collect(),
        controls: program
            .controls
            .iter()
            .filter(|k| reachable.contains(&k.id.0))
            .cloned()
            .collect(),
        contracts: program.contracts.clone(), // always keep
        memories: program
            .memories
            .iter()
            .filter(|m| reachable.contains(&m.id.0))
            .cloned()
            .collect(),
        externs: program
            .externs
            .iter()
            .filter(|x| reachable.contains(&x.id.0))
            .cloned()
            .collect(),
        entry: program.entry.clone(),
    }
}

/// BFS reachability from entry + contracts (mirrors compiler.rs logic).
fn collect_reachable(program: &Program) -> HashSet<String> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    for t in &program.types {
        adj.insert(t.id.0.clone(), refs_for_type(t));
    }
    for r in &program.regions {
        adj.insert(
            r.id.0.clone(),
            r.parent.as_ref().map_or_else(Vec::new, |p| vec![p.0.clone()]),
        );
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

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(program.entry.0.clone());
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
// Reference helpers (mirroring compiler.rs)
// ---------------------------------------------------------------------------

fn refs_from_type_ref(tr: &TypeRef) -> Vec<String> {
    match tr {
        TypeRef::Id { node } => vec![node.0.clone()],
        TypeRef::Builtin { .. } => vec![],
    }
}

fn refs_for_type(t: &TypeDef) -> Vec<String> {
    match &t.body {
        TypeBody::Struct { fields, .. } => fields
            .iter()
            .flat_map(|f| refs_from_type_ref(&f.type_ref))
            .collect(),
        TypeBody::Array { element, .. } => refs_from_type_ref(element),
        TypeBody::Variant { cases } => cases
            .iter()
            .flat_map(|c| refs_from_type_ref(&c.payload))
            .collect(),
        TypeBody::Fn {
            params, result, ..
        } => {
            let mut v: Vec<String> = params.iter().flat_map(refs_from_type_ref).collect();
            v.extend(refs_from_type_ref(result));
            v
        }
        _ => vec![],
    }
}

fn refs_for_compute(c: &ComputeDef) -> Vec<String> {
    match &c.op {
        ComputeOp::Const {
            type_ref, region, ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            if let Some(r) = region {
                v.push(r.0.clone());
            }
            v
        }
        ComputeOp::ConstBytes {
            type_ref, region, ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(region.0.clone());
            v
        }
        ComputeOp::Arith {
            inputs, type_ref, ..
        }
        | ComputeOp::CallPure {
            inputs, type_ref, ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            v
        }
        ComputeOp::Generic {
            inputs,
            type_ref,
            region,
            ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(r) = region {
                v.push(r.0.clone());
            }
            v
        }
        ComputeOp::AtomicLoad {
            source, type_ref, ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(source.0.clone());
            v
        }
        ComputeOp::AtomicStore { target, value, .. } => {
            vec![target.0.clone(), value.0.clone()]
        }
        ComputeOp::AtomicCas {
            target,
            expected,
            desired,
            success,
            failure,
            ..
        } => {
            vec![
                target.0.clone(),
                expected.0.clone(),
                desired.0.clone(),
                success.0.clone(),
                failure.0.clone(),
            ]
        }
    }
}

fn refs_for_effect(e: &EffectDef) -> Vec<String> {
    match &e.op {
        EffectOp::Syscall {
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(s) = success {
                v.push(s.0.clone());
            }
            if let Some(f) = failure {
                v.push(f.0.clone());
            }
            v
        }
        EffectOp::CallExtern {
            target,
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(target.0.clone());
            v.extend(inputs.iter().map(|i| i.0.clone()));
            v.push(success.0.clone());
            v.push(failure.0.clone());
            v
        }
        EffectOp::Generic {
            inputs,
            type_ref,
            success,
            failure,
            ..
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.extend(inputs.iter().map(|i| i.0.clone()));
            if let Some(s) = success {
                v.push(s.0.clone());
            }
            if let Some(f) = failure {
                v.push(f.0.clone());
            }
            v
        }
    }
}

fn refs_for_control(k: &ControlDef) -> Vec<String> {
    match &k.op {
        ControlOp::Seq { steps } => steps.iter().map(|s| s.0.clone()).collect(),
        ControlOp::Branch {
            condition,
            true_branch,
            false_branch,
        } => {
            vec![
                condition.0.clone(),
                true_branch.0.clone(),
                false_branch.0.clone(),
            ]
        }
        ControlOp::Loop {
            condition,
            body,
            state,
            state_type,
            ..
        } => {
            let mut v = vec![condition.0.clone(), body.0.clone(), state.0.clone()];
            v.extend(refs_from_type_ref(state_type));
            v
        }
        ControlOp::Par { branches, .. } => branches.iter().map(|b| b.0.clone()).collect(),
    }
}

fn refs_for_contract(c: &ContractDef) -> Vec<String> {
    vec![c.target.0.clone()]
}

fn refs_for_memory(m: &MemoryDef) -> Vec<String> {
    match &m.op {
        MemoryOp::Alloc { type_ref, region } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(region.0.clone());
            v
        }
        MemoryOp::Load {
            source,
            index,
            type_ref,
        } => {
            let mut v = refs_from_type_ref(type_ref);
            v.push(source.0.clone());
            v.push(index.0.clone());
            v
        }
        MemoryOp::Store {
            target,
            index,
            value,
        } => {
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
// LLVM Pass Manager integration
// ---------------------------------------------------------------------------

/// Run LLVM optimization passes on a module's function.
///
/// This is called from codegen after the IR is generated but before
/// verification and emission. The pass selection mirrors clang's -O levels.
pub fn optimize_llvm_function<'ctx>(
    module: &Module<'ctx>,
    function: FunctionValue<'ctx>,
    opt_level: u8,
) {
    if opt_level == 0 {
        return;
    }

    let fpm: PassManager<FunctionValue<'ctx>> = PassManager::create(module);

    // -O1: basic cleanup
    if opt_level >= 1 {
        fpm.add_instruction_combining_pass();
        fpm.add_reassociate_pass();
        fpm.add_gvn_pass();
        fpm.add_cfg_simplification_pass();
        fpm.add_basic_alias_analysis_pass();
    }

    // -O2: memory and loop optimizations
    if opt_level >= 2 {
        fpm.add_promote_memory_to_register_pass();
        fpm.add_loop_unroll_pass();
        fpm.add_licm_pass();
        fpm.add_tail_call_elimination_pass();
    }

    // -O3: aggressive
    if opt_level >= 3 {
        fpm.add_aggressive_dce_pass();
        fpm.add_merged_load_store_motion_pass();
        fpm.add_dead_store_elimination_pass();
    }

    fpm.initialize();
    fpm.run_on(&function);
    fpm.finalize();
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a minimal program with the given computes and entry.
    fn minimal_program(computes: Vec<ComputeDef>, entry: &str) -> Program {
        Program {
            types: vec![],
            regions: vec![],
            computes,
            effects: vec![],
            controls: vec![],
            contracts: vec![],
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new(entry),
        }
    }

    #[test]
    fn fold_add_constants() {
        let c1 = ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 3 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c2 = ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 4 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c3 = ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "add".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
            },
        };

        let prog = minimal_program(vec![c1, c2, c3], "C:c3");
        let result = fold_constants(&prog);

        assert_eq!(result.constants_folded, 1);
        // c3 should now be a Const with value 7
        let c3_node = result
            .program
            .computes
            .iter()
            .find(|c| c.id.0 == "C:c3")
            .expect("C:c3 should exist");
        match &c3_node.op {
            ComputeOp::Const {
                value: Literal::Integer { value: 7 },
                ..
            } => {} // success
            other => panic!("expected Const(7), got {:?}", other),
        }
    }

    #[test]
    fn fold_mul_constants() {
        let c1 = ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 5 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c2 = ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 6 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c3 = ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "mul".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
            },
        };

        let prog = minimal_program(vec![c1, c2, c3], "C:c3");
        let result = fold_constants(&prog);

        assert_eq!(result.constants_folded, 1);
        let c3_node = result
            .program
            .computes
            .iter()
            .find(|c| c.id.0 == "C:c3")
            .expect("C:c3 should exist");
        match &c3_node.op {
            ComputeOp::Const {
                value: Literal::Integer { value: 30 },
                ..
            } => {}
            other => panic!("expected Const(30), got {:?}", other),
        }
    }

    #[test]
    fn identity_add_zero() {
        let c1 = ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 42 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c2 = ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 0 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c3 = ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "add".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
            },
        };

        let prog = minimal_program(vec![c1, c2, c3], "C:c3");
        let result = remove_identities(&prog);

        assert_eq!(result.identities_removed, 1);
        let c3_node = result
            .program
            .computes
            .iter()
            .find(|c| c.id.0 == "C:c3")
            .expect("C:c3 should exist");
        match &c3_node.op {
            ComputeOp::Const {
                value: Literal::Integer { value: 42 },
                ..
            } => {}
            other => panic!("expected Const(42), got {:?}", other),
        }
    }

    #[test]
    fn identity_mul_one() {
        let c1 = ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 99 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c2 = ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 1 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c3 = ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "mul".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
            },
        };

        let prog = minimal_program(vec![c1, c2, c3], "C:c3");
        let result = remove_identities(&prog);

        assert_eq!(result.identities_removed, 1);
        let c3_node = result
            .program
            .computes
            .iter()
            .find(|c| c.id.0 == "C:c3")
            .expect("C:c3 should exist");
        match &c3_node.op {
            ComputeOp::Const {
                value: Literal::Integer { value: 99 },
                ..
            } => {}
            other => panic!("expected Const(99), got {:?}", other),
        }
    }

    #[test]
    fn no_optimization_when_disabled() {
        let c1 = ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 3 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c2 = ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 4 },
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
                region: None,
            },
        };
        let c3 = ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "add".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "i64".to_string(),
                },
            },
        };

        let prog = minimal_program(vec![c1, c2, c3], "C:c3");
        let config = OptimizationConfig {
            enable_graph_opts: false,
            ..Default::default()
        };
        let result = optimize_graph(&prog, &config);

        assert_eq!(result.stats.constants_folded, 0);
        assert_eq!(result.stats.nodes_before, result.stats.nodes_after);
    }
}
