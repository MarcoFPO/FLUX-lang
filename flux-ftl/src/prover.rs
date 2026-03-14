use std::collections::HashMap;

use serde::Serialize;
use z3::ast::{Ast, Bool, Int};
use z3::{Config, Context, SatResult, Solver};

use crate::ast::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ProofResult {
    pub contract_id: String,
    pub target_id: String,
    pub clause_index: usize,
    pub clause_kind: String,
    pub status: ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample_model: Option<CounterexampleModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofStatus {
    Proven,
    Disproven,
    Unknown,
    Assumed,
    Timeout,
    BmcProven,
    BmcRefuted,
}

/// BMC search strategy for finding the right unrolling depth.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub enum BmcStrategy {
    /// Unroll linearly from 1 to max_depth (original behavior).
    #[default]
    Linear,
    /// Binary search: check max_depth/2, then narrow based on result.
    Binary,
    /// Start small, double on success until max_depth (logarithmic).
    Adaptive,
}

/// Structured counterexample model: variable name to value pairs.
pub type CounterexampleModel = Vec<(String, String)>;

/// Result of proving a clause: (status, counterexample_string, structured_model).
type ClauseResult = (ProofStatus, Option<String>, Option<CounterexampleModel>);

#[derive(Debug, Clone)]
pub struct BmcConfig {
    pub max_depth: u32,
    pub timeout_secs: u64,
    pub strategy: BmcStrategy,
}

impl Default for BmcConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            timeout_secs: 300,
            strategy: BmcStrategy::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProverConfig {
    pub timeout_ms: u32,
    pub bmc_config: Option<BmcConfig>,
}

impl Default for ProverConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            bmc_config: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ProverContext — lookup maps built from the Program AST
// ---------------------------------------------------------------------------

struct ProverContext<'a> {
    consts: HashMap<String, &'a ComputeDef>,
    types: HashMap<String, &'a TypeDef>,
    controls: HashMap<String, &'a ControlDef>,
}

impl<'a> ProverContext<'a> {
    fn from_program(program: &'a Program) -> Self {
        let mut consts = HashMap::new();
        for c in &program.computes {
            consts.insert(c.id.as_str().to_string(), c);
        }
        let mut types = HashMap::new();
        for t in &program.types {
            types.insert(t.id.as_str().to_string(), t);
        }
        let mut controls = HashMap::new();
        for k in &program.controls {
            controls.insert(k.id.as_str().to_string(), k);
        }
        ProverContext { consts, types, controls }
    }
}

// ---------------------------------------------------------------------------
// resolve_type — TypeRef → TypeBody
// ---------------------------------------------------------------------------

fn resolve_type<'a>(ctx: &'a ProverContext, type_ref: &TypeRef) -> Option<&'a TypeBody> {
    match type_ref {
        TypeRef::Id { node } => ctx.types.get(node.as_str()).map(|td| &td.body),
        TypeRef::Builtin { name } => {
            // Synthesize TypeBody for builtin types
            // We return None here and handle builtins via resolve_type_body_for_builtin
            BUILTIN_TYPES.iter().find(|(n, _)| *n == name.as_str()).map(|(_, b)| b)
        }
    }
}

/// Static builtin type definitions for common types.
static BUILTIN_TYPES: &[(&str, TypeBody)] = &[
    ("u8", TypeBody::Integer { bits: 8, signed: false }),
    ("u16", TypeBody::Integer { bits: 16, signed: false }),
    ("u32", TypeBody::Integer { bits: 32, signed: false }),
    ("u64", TypeBody::Integer { bits: 64, signed: false }),
    ("i8", TypeBody::Integer { bits: 8, signed: true }),
    ("i16", TypeBody::Integer { bits: 16, signed: true }),
    ("i32", TypeBody::Integer { bits: 32, signed: true }),
    ("i64", TypeBody::Integer { bits: 64, signed: true }),
    ("bool", TypeBody::Boolean),
];

// ---------------------------------------------------------------------------
// extract_type_ref — get type_ref from a ComputeOp if available
// ---------------------------------------------------------------------------

fn extract_type_ref(op: &ComputeOp) -> Option<&TypeRef> {
    match op {
        ComputeOp::Const { type_ref, .. } => Some(type_ref),
        ComputeOp::ConstBytes { type_ref, .. } => Some(type_ref),
        ComputeOp::Arith { type_ref, .. } => Some(type_ref),
        ComputeOp::CallPure { type_ref, .. } => Some(type_ref),
        ComputeOp::Generic { type_ref, .. } => Some(type_ref),
        ComputeOp::AtomicLoad { type_ref, .. } => Some(type_ref),
        ComputeOp::AtomicStore { .. } => None,
        ComputeOp::AtomicCas { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// collect_symbolic_names — extract symbolic variable names from a formula
// ---------------------------------------------------------------------------

fn collect_symbolic_expr_names(ctx: &ProverContext, expr: &Expr, names: &mut Vec<String>) {
    match expr {
        Expr::FieldAccess { node, fields } => {
            if resolve_field_access(ctx, node.as_str(), fields).is_none() {
                let name = if fields.is_empty() {
                    node.as_str().to_string()
                } else {
                    format!("{}.{}", node.as_str(), fields.join("."))
                };
                names.push(name);
            }
        }
        Expr::Ident { name } => {
            if name != "null" && resolve_field_access(ctx, name, &[]).is_none() {
                names.push(name.clone());
            }
        }
        Expr::Result => names.push("result".to_string()),
        Expr::State => names.push("state".to_string()),
        Expr::BinOp { left, right, .. } => {
            collect_symbolic_expr_names(ctx, left, names);
            collect_symbolic_expr_names(ctx, right, names);
        }
        _ => {}
    }
}

fn collect_symbolic_formula_names(ctx: &ProverContext, formula: &Formula, names: &mut Vec<String>) {
    match formula {
        Formula::Comparison { left, right, .. } => {
            collect_symbolic_expr_names(ctx, left, names);
            collect_symbolic_expr_names(ctx, right, names);
        }
        Formula::And { left, right } | Formula::Or { left, right } => {
            collect_symbolic_formula_names(ctx, left, names);
            collect_symbolic_formula_names(ctx, right, names);
        }
        Formula::Not { inner } => collect_symbolic_formula_names(ctx, inner, names),
        Formula::Forall { range_start, range_end, body, .. } => {
            collect_symbolic_expr_names(ctx, range_start, names);
            collect_symbolic_expr_names(ctx, range_end, names);
            collect_symbolic_formula_names(ctx, body, names);
        }
        Formula::FieldAccess { node, fields } => {
            if resolve_field_access(ctx, node.as_str(), fields).is_none() {
                let name = if fields.is_empty() {
                    node.as_str().to_string()
                } else {
                    format!("{}.{}", node.as_str(), fields.join("."))
                };
                names.push(name);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// collect_type_constraints — generate Z3 constraints from type information
// ---------------------------------------------------------------------------

fn collect_type_constraints<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    contract: &ContractDef,
) -> Vec<Bool<'z>> {
    let mut constraints = Vec::new();

    // Collect all symbolic variable names from all clauses in this contract
    let mut symbolic_names = Vec::new();
    for clause in &contract.clauses {
        let formula = match clause {
            ContractClause::Pre { formula } => formula,
            ContractClause::Post { formula } => formula,
            ContractClause::Invariant { formula } => formula,
            ContractClause::Assume { formula } => formula,
        };
        collect_symbolic_formula_names(prover_ctx, formula, &mut symbolic_names);
    }
    symbolic_names.sort();
    symbolic_names.dedup();

    for sym_name in &symbolic_names {
        // Case 1: C-Node field access like "C:s2_load.val"
        if let Some(dot_pos) = sym_name.find('.') {
            let node_id = &sym_name[..dot_pos];
            let field = &sym_name[dot_pos + 1..];

            if let Some(cdef) = prover_ctx.consts.get(node_id)
                && let Some(type_ref) = extract_type_ref(&cdef.op)
                && field == "val"
            {
                add_type_constraints_for_var(
                    z3_ctx, prover_ctx, sym_name, type_ref, &mut constraints,
                );
            }
        }

        // Case 2: "state.length" or "state.score" — resolve via loop state_type
        if let Some(field_name) = sym_name.strip_prefix("state.") {
            // Find the contract's target and look for a loop with state_type
            if let Some(state_type_ref) = find_loop_state_type(prover_ctx, contract)
                && let Some(type_body) = resolve_type(prover_ctx, state_type_ref)
            {
                add_struct_field_constraints(
                    z3_ctx, prover_ctx, sym_name, field_name, type_body, &mut constraints,
                );
            }
        }
    }

    constraints
}

/// Find the state_type of a loop that the contract targets (directly or indirectly).
fn find_loop_state_type<'a>(
    prover_ctx: &'a ProverContext,
    contract: &ContractDef,
) -> Option<&'a TypeRef> {
    let target = contract.target.as_str();
    // Direct target is a K-Node loop
    if let Some(kdef) = prover_ctx.controls.get(target)
        && let ControlOp::Loop { state_type, .. } = &kdef.op
    {
        return Some(state_type);
    }
    None
}

/// Add Z3 constraints for a variable based on its TypeRef.
fn add_type_constraints_for_var<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    var_name: &str,
    type_ref: &TypeRef,
    constraints: &mut Vec<Bool<'z>>,
) {
    if let Some(type_body) = resolve_type(prover_ctx, type_ref) {
        let z3_var = Int::new_const(z3_ctx, var_name);
        match type_body {
            TypeBody::Integer { bits, signed } => {
                if !signed {
                    // unsigned: 0 <= var < 2^bits
                    constraints.push(z3_var.ge(&Int::from_i64(z3_ctx, 0)));
                    if *bits < 64 {
                        let max = 1i64 << bits;
                        constraints.push(z3_var.lt(&Int::from_i64(z3_ctx, max)));
                    }
                } else {
                    // signed: -2^(bits-1) <= var < 2^(bits-1)
                    let half = 1i64 << (bits - 1);
                    constraints.push(z3_var.ge(&Int::from_i64(z3_ctx, -half)));
                    constraints.push(z3_var.lt(&Int::from_i64(z3_ctx, half)));
                }
            }
            TypeBody::Boolean => {
                constraints.push(z3_var.ge(&Int::from_i64(z3_ctx, 0)));
                constraints.push(z3_var.le(&Int::from_i64(z3_ctx, 1)));
            }
            _ => {}
        }
    }
}

/// Add constraints for a struct field access like "state.length".
/// If the struct has an array field whose associated length field can be identified,
/// constrain the length to 0..=max_length.
fn add_struct_field_constraints<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    var_name: &str,
    field_name: &str,
    struct_body: &TypeBody,
    constraints: &mut Vec<Bool<'z>>,
) {
    if let TypeBody::Struct { fields, .. } = struct_body {
        // First, find the field and add type-based constraints from its own type
        let mut field_type_ref = None;
        for sf in fields {
            if sf.name == field_name {
                field_type_ref = Some(&sf.type_ref);
                break;
            }
        }
        if let Some(tr) = field_type_ref {
            add_type_constraints_for_var(z3_ctx, prover_ctx, var_name, tr, constraints);
        }

        // Second, if the field is named "length" or "len", look for an array field
        // in the same struct and constrain by max_length.
        if field_name == "length" || field_name == "len" {
            for sf in fields {
                if let Some(type_body) = resolve_type(prover_ctx, &sf.type_ref)
                    && let TypeBody::Array { max_length, .. } = type_body
                {
                    let z3_var = Int::new_const(z3_ctx, var_name);
                    constraints.push(z3_var.ge(&Int::from_i64(z3_ctx, 0)));
                    constraints.push(z3_var.le(&Int::from_i64(z3_ctx, *max_length as i64)));
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// collect_assume_axioms — gather Assume formulas preceding the clause
// ---------------------------------------------------------------------------

fn collect_assume_axioms<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    clauses: &[ContractClause],
    current_idx: usize,
) -> Vec<Bool<'z>> {
    let mut axioms = Vec::new();
    for clause in &clauses[..current_idx] {
        if let ContractClause::Assume { formula } = clause
            && let Some(z3_f) = translate_formula(z3_ctx, prover_ctx, formula)
        {
            axioms.push(z3_f);
        }
    }
    axioms
}

// ---------------------------------------------------------------------------
// resolve_field_access — C:c2.val → concrete i64 value
// ---------------------------------------------------------------------------

fn resolve_field_access(
    ctx: &ProverContext,
    node_id: &str,
    fields: &[String],
) -> Option<i64> {
    let cdef = ctx.consts.get(node_id)?;
    match &cdef.op {
        ComputeOp::Const { value, .. } => {
            // .val or empty fields → extract the literal value
            if fields.is_empty() || (fields.len() == 1 && fields[0] == "val") {
                match value {
                    Literal::Integer { value } => Some(*value),
                    Literal::Bool { value } => Some(if *value { 1 } else { 0 }),
                    _ => None,
                }
            } else {
                None
            }
        }
        ComputeOp::ConstBytes { value, .. } => {
            // ConstBytes without field access → treat as non-null pointer (address 1)
            if fields.is_empty() {
                Some(if value.is_empty() { 0 } else { 1 })
            } else if fields.len() == 1 && fields[0] == "len" {
                Some(value.len() as i64)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// translate_expr — Expr AST → Z3 Int expression
// ---------------------------------------------------------------------------

fn translate_expr<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    expr: &Expr,
) -> Option<Int<'z>> {
    match expr {
        Expr::IntLit { value } => Some(Int::from_i64(z3_ctx, *value)),

        Expr::FieldAccess { node, fields } => {
            match resolve_field_access(prover_ctx, node.as_str(), fields) {
                Some(v) => Some(Int::from_i64(z3_ctx, v)),
                None => {
                    // Symbolic: unresolvable field access
                    let name = if fields.is_empty() {
                        node.as_str().to_string()
                    } else {
                        format!("{}.{}", node.as_str(), fields.join("."))
                    };
                    Some(Int::new_const(z3_ctx, name.as_str()))
                }
            }
        }

        Expr::Ident { name } => {
            if name == "null" {
                Some(Int::from_i64(z3_ctx, 0))
            } else if let Some(val) = resolve_field_access(prover_ctx, name, &[]) {
                Some(Int::from_i64(z3_ctx, val))
            } else {
                Some(Int::new_const(z3_ctx, name.as_str()))
            }
        }

        Expr::Result => Some(Int::new_const(z3_ctx, "result")),
        Expr::State => Some(Int::new_const(z3_ctx, "state")),

        Expr::BinOp { left, op, right } => {
            let l = translate_expr(z3_ctx, prover_ctx, left)?;
            let r = translate_expr(z3_ctx, prover_ctx, right)?;
            Some(match op {
                ArithBinOp::Add => Int::add(z3_ctx, &[&l, &r]),
                ArithBinOp::Sub => Int::sub(z3_ctx, &[&l, &r]),
                ArithBinOp::Mul => Int::mul(z3_ctx, &[&l, &r]),
                ArithBinOp::Div => l.div(&r),
                ArithBinOp::Mod => l.modulo(&r),
            })
        }

        Expr::EmptySet => Some(Int::from_i64(z3_ctx, 0)),

        Expr::PredicateCall { .. } => None,
        Expr::FloatLit { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// translate_formula — Formula AST → Z3 Bool expression
// ---------------------------------------------------------------------------

fn translate_formula<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    formula: &Formula,
) -> Option<Bool<'z>> {
    match formula {
        Formula::BoolLit { value } => Some(Bool::from_bool(z3_ctx, *value)),

        Formula::And { left, right } => {
            let l = translate_formula(z3_ctx, prover_ctx, left)?;
            let r = translate_formula(z3_ctx, prover_ctx, right)?;
            Some(Bool::and(z3_ctx, &[&l, &r]))
        }

        Formula::Or { left, right } => {
            let l = translate_formula(z3_ctx, prover_ctx, left)?;
            let r = translate_formula(z3_ctx, prover_ctx, right)?;
            Some(Bool::or(z3_ctx, &[&l, &r]))
        }

        Formula::Not { inner } => {
            let i = translate_formula(z3_ctx, prover_ctx, inner)?;
            Some(i.not())
        }

        Formula::Comparison { left, op, right } => {
            let l = translate_expr(z3_ctx, prover_ctx, left)?;
            let r = translate_expr(z3_ctx, prover_ctx, right)?;
            Some(match op {
                CmpOp::Eq => l._eq(&r),
                CmpOp::Neq => l._eq(&r).not(),
                CmpOp::Lt => l.lt(&r),
                CmpOp::Lte => l.le(&r),
                CmpOp::Gt => l.gt(&r),
                CmpOp::Gte => l.ge(&r),
            })
        }

        Formula::Forall { var, range_start, range_end, body } => {
            let z3_var = Int::new_const(z3_ctx, var.as_str());
            let start = translate_expr(z3_ctx, prover_ctx, range_start)?;
            let end = translate_expr(z3_ctx, prover_ctx, range_end)?;
            let body_z3 = translate_formula(z3_ctx, prover_ctx, body)?;

            // forall var: (start <= var AND var < end) => body
            let range_constraint = Bool::and(z3_ctx, &[&z3_var.ge(&start), &z3_var.lt(&end)]);
            let implication = range_constraint.implies(&body_z3);

            let bound = z3::ast::Dynamic::from_ast(&z3_var);
            Some(z3::ast::forall_const(z3_ctx, &[&bound], &[], &implication))
        }

        Formula::FieldAccess { node, fields } => {
            // A FieldAccess at formula level is treated as a boolean check
            // e.g., C:c_alsa_path != null is Comparison, but bare field → symbolic bool
            match resolve_field_access(prover_ctx, node.as_str(), fields) {
                Some(v) => Some(Bool::from_bool(z3_ctx, v != 0)),
                None => {
                    let name = if fields.is_empty() {
                        node.as_str().to_string()
                    } else {
                        format!("{}.{}", node.as_str(), fields.join("."))
                    };
                    Some(Bool::new_const(z3_ctx, name.as_str()))
                }
            }
        }

        Formula::PredicateCall { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// BMC — Bounded Model Checking: unfold Forall quantifiers to depth k
// ---------------------------------------------------------------------------

/// Translate a formula for BMC by unfolding Forall quantifiers into conjunctions.
fn translate_formula_bmc<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    formula: &Formula,
    max_depth: u32,
) -> Option<Bool<'z>> {
    match formula {
        Formula::Forall { var, range_start, range_end, body } => {
            let start_val = eval_expr_to_i64(prover_ctx, range_start);
            let end_val = eval_expr_to_i64(prover_ctx, range_end);

            match (start_val, end_val) {
                (Some(s), Some(e)) => {
                    let range_len = if e > s { (e - s) as u32 } else { 0 };
                    let depth = std::cmp::min(range_len, max_depth);
                    let mut conjuncts: Vec<Bool<'z>> = Vec::new();
                    for i in 0..depth {
                        let val = s + i as i64;
                        let substituted = substitute_var_in_formula(body, var, val);
                        if let Some(b) = translate_formula_bmc(z3_ctx, prover_ctx, &substituted, max_depth) {
                            conjuncts.push(b);
                        } else {
                            return None;
                        }
                    }
                    if conjuncts.is_empty() {
                        Some(Bool::from_bool(z3_ctx, true))
                    } else {
                        let refs: Vec<&Bool<'z>> = conjuncts.iter().collect();
                        Some(Bool::and(z3_ctx, &refs))
                    }
                }
                _ => {
                    // Cannot resolve range bounds -- fall back to regular translation
                    translate_formula(z3_ctx, prover_ctx, formula)
                }
            }
        }

        Formula::And { left, right } => {
            let l = translate_formula_bmc(z3_ctx, prover_ctx, left, max_depth)?;
            let r = translate_formula_bmc(z3_ctx, prover_ctx, right, max_depth)?;
            Some(Bool::and(z3_ctx, &[&l, &r]))
        }

        Formula::Or { left, right } => {
            let l = translate_formula_bmc(z3_ctx, prover_ctx, left, max_depth)?;
            let r = translate_formula_bmc(z3_ctx, prover_ctx, right, max_depth)?;
            Some(Bool::or(z3_ctx, &[&l, &r]))
        }

        Formula::Not { inner } => {
            let i = translate_formula_bmc(z3_ctx, prover_ctx, inner, max_depth)?;
            Some(i.not())
        }

        _ => translate_formula(z3_ctx, prover_ctx, formula),
    }
}

/// Try to evaluate an expression to a concrete i64 value.
fn eval_expr_to_i64(prover_ctx: &ProverContext, expr: &Expr) -> Option<i64> {
    match expr {
        Expr::IntLit { value } => Some(*value),
        Expr::FieldAccess { node, fields } => resolve_field_access(prover_ctx, node.as_str(), fields),
        Expr::Ident { name } => {
            if name == "null" {
                Some(0)
            } else {
                resolve_field_access(prover_ctx, name, &[])
            }
        }
        Expr::BinOp { left, op, right } => {
            let l = eval_expr_to_i64(prover_ctx, left)?;
            let r = eval_expr_to_i64(prover_ctx, right)?;
            match op {
                ArithBinOp::Add => Some(l + r),
                ArithBinOp::Sub => Some(l - r),
                ArithBinOp::Mul => Some(l * r),
                ArithBinOp::Div => {
                    if r == 0 { None } else { Some(l / r) }
                }
                ArithBinOp::Mod => {
                    if r == 0 { None } else { Some(l % r) }
                }
            }
        }
        _ => None,
    }
}

/// Substitute all occurrences of a variable name in a formula with a concrete integer value.
fn substitute_var_in_formula(formula: &Formula, var: &str, value: i64) -> Formula {
    match formula {
        Formula::Comparison { left, op, right } => Formula::Comparison {
            left: substitute_var_in_expr(left, var, value),
            op: op.clone(),
            right: substitute_var_in_expr(right, var, value),
        },
        Formula::And { left, right } => Formula::And {
            left: Box::new(substitute_var_in_formula(left, var, value)),
            right: Box::new(substitute_var_in_formula(right, var, value)),
        },
        Formula::Or { left, right } => Formula::Or {
            left: Box::new(substitute_var_in_formula(left, var, value)),
            right: Box::new(substitute_var_in_formula(right, var, value)),
        },
        Formula::Not { inner } => Formula::Not {
            inner: Box::new(substitute_var_in_formula(inner, var, value)),
        },
        Formula::Forall { var: inner_var, range_start, range_end, body } => {
            if inner_var == var {
                formula.clone()
            } else {
                Formula::Forall {
                    var: inner_var.clone(),
                    range_start: substitute_var_in_expr(range_start, var, value),
                    range_end: substitute_var_in_expr(range_end, var, value),
                    body: Box::new(substitute_var_in_formula(body, var, value)),
                }
            }
        }
        _ => formula.clone(),
    }
}

/// Substitute a variable name in an expression with a concrete integer value.
fn substitute_var_in_expr(expr: &Expr, var: &str, value: i64) -> Expr {
    match expr {
        Expr::Ident { name } if name == var => Expr::IntLit { value },
        Expr::BinOp { left, op, right } => Expr::BinOp {
            left: Box::new(substitute_var_in_expr(left, var, value)),
            op: op.clone(),
            right: Box::new(substitute_var_in_expr(right, var, value)),
        },
        _ => expr.clone(),
    }
}

/// Extract structured counterexample model from a Z3 model.
/// Returns (variable_name, value) pairs parsed from the model's string representation.
/// The Z3 model Display format uses lines like: `var_name -> value`
fn extract_counterexample_model(model: &z3::Model) -> Vec<(String, String)> {
    let model_str = format!("{}", model);
    let mut pairs = Vec::new();
    for line in model_str.lines() {
        let line = line.trim();
        if let Some(arrow_pos) = line.find(" -> ") {
            let name = line[..arrow_pos].trim().to_string();
            let value = line[arrow_pos + 4..].trim().to_string();
            if !name.is_empty() && !value.is_empty() {
                pairs.push((name, value));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

/// Generate depths to check based on BMC strategy.
fn bmc_strategy_depths(strategy: &BmcStrategy, max_depth: u32) -> Vec<u32> {
    match strategy {
        BmcStrategy::Linear => {
            // Single check at max_depth (existing behavior)
            vec![max_depth]
        }
        BmcStrategy::Binary => {
            // Binary search: check at midpoints, converging toward max_depth
            let mut depths = Vec::new();
            let mut lo = 1u32;
            while lo <= max_depth {
                let mid = lo + (max_depth - lo) / 2;
                depths.push(mid);
                if mid == max_depth {
                    break;
                }
                lo = mid + 1;
            }
            if depths.last() != Some(&max_depth) {
                depths.push(max_depth);
            }
            depths
        }
        BmcStrategy::Adaptive => {
            // Start at 1, double until max_depth
            let mut depths = Vec::new();
            let mut d = 1u32;
            while d < max_depth {
                depths.push(d);
                d = d.saturating_mul(2);
            }
            depths.push(max_depth);
            depths
        }
    }
}

/// BMC check for a single clause: unfold quantifiers and check with Z3.
/// Uses the configured strategy to determine which depths to check.
fn bmc_check_clause<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    clause: &ContractClause,
    clause_idx: usize,
    contract: &ContractDef,
    type_constraints: &[Bool<'z>],
    bmc_config: &BmcConfig,
) -> ClauseResult {
    let formula = match clause {
        ContractClause::Pre { formula } => formula,
        ContractClause::Post { formula } => formula,
        ContractClause::Invariant { formula } => formula,
        ContractClause::Assume { .. } => return (ProofStatus::Assumed, None, None),
    };

    let timeout_ms = (bmc_config.timeout_secs * 1000) as u32;
    let assume_axioms = collect_assume_axioms(z3_ctx, prover_ctx, &contract.clauses, clause_idx);
    let depths = bmc_strategy_depths(&bmc_config.strategy, bmc_config.max_depth);

    for &depth in &depths {
        let z3_formula = match translate_formula_bmc(z3_ctx, prover_ctx, formula, depth) {
            Some(f) => f,
            None => return (ProofStatus::Unknown, Some("BMC: untranslatable formula".into()), None),
        };

        let solver = Solver::new(z3_ctx);
        solver.set_params(&{
            let mut params = z3::Params::new(z3_ctx);
            params.set_u32("timeout", timeout_ms);
            params
        });

        for tc in type_constraints {
            solver.assert(tc);
        }
        for axiom in &assume_axioms {
            solver.assert(axiom);
        }
        solver.assert(&z3_formula.not());

        match solver.check() {
            SatResult::Unsat => {
                if depth == bmc_config.max_depth {
                    return (ProofStatus::BmcProven, None, None);
                }
                // Proven at lower depth, continue to max_depth for full coverage
                continue;
            }
            SatResult::Sat => {
                let (ce, model) = solver
                    .get_model()
                    .map(|m| {
                        let ce_str = format!("{}", m);
                        let model_pairs = extract_counterexample_model(&m);
                        (
                            if ce_str.is_empty() { None } else { Some(ce_str) },
                            if model_pairs.is_empty() { None } else { Some(model_pairs) },
                        )
                    })
                    .unwrap_or((None, None));
                return (ProofStatus::BmcRefuted, ce, model);
            }
            SatResult::Unknown => {
                if depth == bmc_config.max_depth {
                    let reason = solver.get_reason_unknown().unwrap_or_default();
                    return (ProofStatus::Unknown, Some(format!("BMC: {}", reason)), None);
                }
                continue;
            }
        }
    }

    // Fallback: should not normally be reached since depths always ends with max_depth
    (ProofStatus::Unknown, Some("BMC: no result".into()), None)
}

// ---------------------------------------------------------------------------
// Invariant strengthening — add known const values as constraints
// ---------------------------------------------------------------------------

/// Collect known const values from the program context for invariant strengthening.
fn collect_const_bounds<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
) -> Vec<Bool<'z>> {
    let mut bounds = Vec::new();
    for (id, cdef) in &prover_ctx.consts {
        if let ComputeOp::Const { value: Literal::Integer { value }, .. } = &cdef.op {
            let var_name = format!("{}.val", id);
            let z3_var = Int::new_const(z3_ctx, var_name.as_str());
            bounds.push(z3_var._eq(&Int::from_i64(z3_ctx, *value)));
        }
    }
    bounds
}

// ---------------------------------------------------------------------------
// prove_clause -- negation check pattern with type constraints and assumptions
// ---------------------------------------------------------------------------

fn prove_clause<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    clause: &ContractClause,
    clause_idx: usize,
    contract: &ContractDef,
    type_constraints: &[Bool<'z>],
    config: &ProverConfig,
) -> ClauseResult {
    if let ContractClause::Assume { .. } = clause {
        return (ProofStatus::Assumed, None, None);
    }

    let formula = match clause {
        ContractClause::Pre { formula } => formula,
        ContractClause::Post { formula } => formula,
        ContractClause::Invariant { formula } => formula,
        ContractClause::Assume { .. } => unreachable!(),
    };

    let z3_formula = match translate_formula(z3_ctx, prover_ctx, formula) {
        Some(f) => f,
        None => return (ProofStatus::Unknown, Some("untranslatable formula".into()), None),
    };

    let solver = Solver::new(z3_ctx);
    solver.set_params(&{
        let mut params = z3::Params::new(z3_ctx);
        params.set_u32("timeout", config.timeout_ms);
        params
    });

    // Assert type constraints to narrow symbolic variable ranges
    for tc in type_constraints {
        solver.assert(tc);
    }

    // Assert Assume clauses preceding this clause as axioms
    let assume_axioms = collect_assume_axioms(z3_ctx, prover_ctx, &contract.clauses, clause_idx);
    for axiom in &assume_axioms {
        solver.assert(axiom);
    }

    // Negation check: assert NOT(formula), check SAT
    // UNSAT → formula always holds → PROVEN
    // SAT → formula can be violated → DISPROVEN
    solver.assert(&z3_formula.not());

    match solver.check() {
        SatResult::Unsat => (ProofStatus::Proven, None, None),
        SatResult::Sat => {
            let (ce, model) = solver
                .get_model()
                .map(|m| {
                    let ce_str = format!("{}", m);
                    let model_pairs = extract_counterexample_model(&m);
                    (
                        if ce_str.is_empty() { None } else { Some(ce_str) },
                        if model_pairs.is_empty() { None } else { Some(model_pairs) },
                    )
                })
                .unwrap_or((None, None));
            (ProofStatus::Disproven, ce, model)
        }
        SatResult::Unknown => {
            let reason = solver.get_reason_unknown().unwrap_or_default();
            let z3_result: ClauseResult =
                if reason.contains("timeout") {
                    (ProofStatus::Timeout, Some(reason), None)
                } else {
                    (ProofStatus::Unknown, Some(reason), None)
                };

            // Invariant strengthening: for Invariant clauses with Unknown,
            // try again with additional const bounds from program context
            if z3_result.0 == ProofStatus::Unknown
                && matches!(clause, ContractClause::Invariant { .. })
            {
                let const_bounds = collect_const_bounds(z3_ctx, prover_ctx);
                if !const_bounds.is_empty() {
                    let strengthened_solver = Solver::new(z3_ctx);
                    strengthened_solver.set_params(&{
                        let mut params = z3::Params::new(z3_ctx);
                        params.set_u32("timeout", config.timeout_ms);
                        params
                    });
                    for tc in type_constraints {
                        strengthened_solver.assert(tc);
                    }
                    for axiom in &assume_axioms {
                        strengthened_solver.assert(axiom);
                    }
                    for bound in &const_bounds {
                        strengthened_solver.assert(bound);
                    }
                    if let Some(ref f) = translate_formula(z3_ctx, prover_ctx, formula) {
                        strengthened_solver.assert(&f.not());
                        match strengthened_solver.check() {
                            SatResult::Unsat => return (ProofStatus::Proven, None, None),
                            SatResult::Sat => {
                                let (ce, model) = strengthened_solver
                                    .get_model()
                                    .map(|m| {
                                        let ce_str = format!("{}", m);
                                        let model_pairs = extract_counterexample_model(&m);
                                        (
                                            if ce_str.is_empty() { None } else { Some(ce_str) },
                                            if model_pairs.is_empty() { None } else { Some(model_pairs) },
                                        )
                                    })
                                    .unwrap_or((None, None));
                                return (ProofStatus::Disproven, ce, model);
                            }
                            SatResult::Unknown => {
                                // Still unknown, fall through to BMC
                            }
                        }
                    }
                }
            }

            // BMC fallback: if Z3 returned Unknown and BMC is configured, try BMC
            if z3_result.0 == ProofStatus::Unknown
                && let Some(bmc_cfg) = &config.bmc_config
            {
                return bmc_check_clause(
                    z3_ctx,
                    prover_ctx,
                    clause,
                    clause_idx,
                    contract,
                    type_constraints,
                    bmc_cfg,
                );
            }

            z3_result
        }
    }
}

// ---------------------------------------------------------------------------
// prove_contracts — public entry point
// ---------------------------------------------------------------------------

pub fn prove_contracts(program: &Program, config: &ProverConfig) -> Vec<ProofResult> {
    let prover_ctx = ProverContext::from_program(program);

    let z3_cfg = Config::new();
    let z3_ctx = Context::new(&z3_cfg);

    let mut results = Vec::new();

    for contract in &program.contracts {
        let is_extern = contract.trust.as_ref() == Some(&TrustLevel::Extern);

        // Compute type constraints once per contract
        let type_constraints = if is_extern {
            vec![]
        } else {
            collect_type_constraints(&z3_ctx, &prover_ctx, contract)
        };

        for (idx, clause) in contract.clauses.iter().enumerate() {
            let kind = match clause {
                ContractClause::Pre { .. } => "pre",
                ContractClause::Post { .. } => "post",
                ContractClause::Invariant { .. } => "invariant",
                ContractClause::Assume { .. } => "assume",
            };

            let (status, counterexample, counterexample_model) = if is_extern {
                (ProofStatus::Assumed, None, None)
            } else {
                prove_clause(
                    &z3_ctx,
                    &prover_ctx,
                    clause,
                    idx,
                    contract,
                    &type_constraints,
                    config,
                )
            };

            results.push(ProofResult {
                contract_id: contract.id.as_str().to_string(),
                target_id: contract.target.as_str().to_string(),
                clause_index: idx,
                clause_kind: kind.to_string(),
                status,
                counterexample,
                counterexample_model,
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_program_with_types(
        contracts: Vec<ContractDef>,
        computes: Vec<ComputeDef>,
        types: Vec<TypeDef>,
    ) -> Program {
        Program {
            types,
            regions: vec![],
            computes,
            effects: vec![],
            controls: vec![],
            contracts,
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new("K:f1"),
        }
    }

    fn make_program_full(
        contracts: Vec<ContractDef>,
        computes: Vec<ComputeDef>,
        types: Vec<TypeDef>,
        controls: Vec<ControlDef>,
    ) -> Program {
        Program {
            types,
            regions: vec![],
            computes,
            effects: vec![],
            controls,
            contracts,
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new("K:f1"),
        }
    }

    fn make_program(contracts: Vec<ContractDef>, computes: Vec<ComputeDef>) -> Program {
        make_program_with_types(contracts, computes, vec![])
    }

    fn int_const(id: &str, val: i64) -> ComputeDef {
        ComputeDef {
            id: NodeRef::new(id),
            op: ComputeOp::Const {
                value: Literal::Integer { value: val },
                type_ref: TypeRef::Builtin { name: "u64".into() },
                region: None,
            },
        }
    }

    #[test]
    fn test_proven_literal_eq() {
        // C:c1.val == 42, where C:c1 = const { value: 42 }
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:c1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Eq,
                    right: Expr::IntLit { value: 42 },
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![int_const("C:c1", 42)]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_disproven_literal_neq() {
        // C:c1.val == 99, where C:c1 = const { value: 42 }
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:c1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Eq,
                    right: Expr::IntLit { value: 99 },
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![int_const("C:c1", 42)]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Disproven);
    }

    #[test]
    fn test_assumed_clause() {
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Assume {
                formula: Formula::BoolLit { value: true },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Assumed);
    }

    #[test]
    fn test_extern_trust_all_assumed() {
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![
                ContractClause::Assume {
                    formula: Formula::BoolLit { value: true },
                },
                ContractClause::Post {
                    formula: Formula::Comparison {
                        left: Expr::Result,
                        op: CmpOp::Neq,
                        right: Expr::IntLit { value: 0 },
                    },
                },
            ],
            trust: Some(TrustLevel::Extern),
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == ProofStatus::Assumed));
    }

    #[test]
    fn test_predicate_call_unknown() {
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f1"),
            clauses: vec![ContractClause::Invariant {
                formula: Formula::PredicateCall {
                    name: "all_accesses_atomic".into(),
                    args: vec![],
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Unknown);
    }

    #[test]
    fn test_bool_lit_true_proven() {
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::BoolLit { value: true },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_bool_lit_false_disproven() {
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::BoolLit { value: false },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Disproven);
    }

    #[test]
    fn test_gt_comparison() {
        // C:c1.val > 0, where C:c1 = const { value: 4096 }
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:c1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Gt,
                    right: Expr::IntLit { value: 0 },
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![int_const("C:c1", 4096)]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_lte_comparison() {
        // C:c1.val <= 4096, where C:c1 = const { value: 10 }
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:c1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Lte,
                    right: Expr::IntLit { value: 4096 },
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![int_const("C:c1", 10)]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    // -----------------------------------------------------------------------
    // Phase 4: Type constraint propagation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_unsigned_atomic_load_gte_zero_proven() {
        // C:s1.val >= 0 where C:s1 is atomic_load with type T:a1 (u64)
        let u64_type = TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 64, signed: false },
        };
        let atomic_load = ComputeDef {
            id: NodeRef::new("C:s1"),
            op: ComputeOp::AtomicLoad {
                source: NodeRef::new("M:g1"),
                order: MemoryOrder::Acquire,
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
            },
        };
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:s1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Gte,
                    right: Expr::IntLit { value: 0 },
                },
            }],
            trust: None,
        };

        let program = make_program_with_types(
            vec![contract],
            vec![atomic_load],
            vec![u64_type],
        );
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_signed_integer_range_constraint() {
        // C:s1.val >= -128 AND C:s1.val < 128 where C:s1 has type i8
        let i8_type = TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 8, signed: true },
        };
        let compute = ComputeDef {
            id: NodeRef::new("C:s1"),
            op: ComputeOp::AtomicLoad {
                source: NodeRef::new("M:g1"),
                order: MemoryOrder::Acquire,
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
            },
        };
        // C:s1.val >= -128 should be proven
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:s1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Gte,
                    right: Expr::IntLit { value: -128 },
                },
            }],
            trust: None,
        };

        let program = make_program_with_types(
            vec![contract],
            vec![compute],
            vec![i8_type],
        );
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_unsigned_out_of_range_disproven() {
        // C:s1.val == -1 where C:s1 is u8 → DISPROVEN (u8 >= 0)
        let u8_type = TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 8, signed: false },
        };
        let compute = ComputeDef {
            id: NodeRef::new("C:s1"),
            op: ComputeOp::AtomicLoad {
                source: NodeRef::new("M:g1"),
                order: MemoryOrder::Acquire,
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
            },
        };
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:s1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Eq,
                    right: Expr::IntLit { value: -1 },
                },
            }],
            trust: None,
        };

        let program = make_program_with_types(
            vec![contract],
            vec![compute],
            vec![u8_type],
        );
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Disproven);
    }

    #[test]
    fn test_builtin_type_constraint() {
        // C:s1.val >= 0 where C:s1 has builtin type "u32"
        let compute = ComputeDef {
            id: NodeRef::new("C:s1"),
            op: ComputeOp::AtomicLoad {
                source: NodeRef::new("M:g1"),
                order: MemoryOrder::Acquire,
                type_ref: TypeRef::Builtin { name: "u32".into() },
            },
        };
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f1"),
            clauses: vec![ContractClause::Pre {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("C:s1"),
                        fields: vec!["val".into()],
                    },
                    op: CmpOp::Gte,
                    right: Expr::IntLit { value: 0 },
                },
            }],
            trust: None,
        };

        let program = make_program(vec![contract], vec![compute]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_struct_array_length_constraint() {
        // state.length <= 800 where state_type is a struct with an array max_length 800
        let pos_type = TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 32, signed: true },
        };
        let array_type = TypeDef {
            id: NodeRef::new("T:a2"),
            body: TypeBody::Array {
                element: TypeRef::Id { node: NodeRef::new("T:a1") },
                max_length: 800,
                constraint: None,
            },
        };
        let state_type = TypeDef {
            id: NodeRef::new("T:a3"),
            body: TypeBody::Struct {
                fields: vec![
                    StructField {
                        name: "snake".into(),
                        type_ref: TypeRef::Id { node: NodeRef::new("T:a2") },
                    },
                    StructField {
                        name: "length".into(),
                        type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
                    },
                ],
                layout: Layout::Optimal,
            },
        };
        let loop_control = ControlDef {
            id: NodeRef::new("K:f_loop"),
            op: ControlOp::Loop {
                condition: NodeRef::new("C:c1"),
                body: NodeRef::new("K:f2"),
                state: NodeRef::new("M:g1"),
                state_type: TypeRef::Id { node: NodeRef::new("T:a3") },
            },
        };
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("K:f_loop"),
            clauses: vec![ContractClause::Invariant {
                formula: Formula::Comparison {
                    left: Expr::FieldAccess {
                        node: NodeRef::new("state"),
                        fields: vec!["length".into()],
                    },
                    op: CmpOp::Lte,
                    right: Expr::IntLit { value: 800 },
                },
            }],
            trust: None,
        };

        let program = make_program_full(
            vec![contract],
            vec![],
            vec![pos_type, array_type, state_type],
            vec![loop_control],
        );
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ProofStatus::Proven);
    }

    #[test]
    fn test_assume_clause_as_axiom() {
        // Assume: result >= 0, then Post: result >= 0 → PROVEN
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![
                ContractClause::Assume {
                    formula: Formula::Comparison {
                        left: Expr::Result,
                        op: CmpOp::Gte,
                        right: Expr::IntLit { value: 0 },
                    },
                },
                ContractClause::Post {
                    formula: Formula::Comparison {
                        left: Expr::Result,
                        op: CmpOp::Gte,
                        right: Expr::IntLit { value: 0 },
                    },
                },
            ],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, ProofStatus::Assumed);
        assert_eq!(results[1].status, ProofStatus::Proven);
    }

    #[test]
    fn test_assume_constrains_symbolic_result() {
        // Assume: result >= 0 AND result <= 100, then Post: result <= 200 → PROVEN
        let contract = ContractDef {
            id: NodeRef::new("V:e1"),
            target: NodeRef::new("E:d1"),
            clauses: vec![
                ContractClause::Assume {
                    formula: Formula::And {
                        left: Box::new(Formula::Comparison {
                            left: Expr::Result,
                            op: CmpOp::Gte,
                            right: Expr::IntLit { value: 0 },
                        }),
                        right: Box::new(Formula::Comparison {
                            left: Expr::Result,
                            op: CmpOp::Lte,
                            right: Expr::IntLit { value: 100 },
                        }),
                    },
                },
                ContractClause::Post {
                    formula: Formula::Comparison {
                        left: Expr::Result,
                        op: CmpOp::Lte,
                        right: Expr::IntLit { value: 200 },
                    },
                },
            ],
            trust: None,
        };

        let program = make_program(vec![contract], vec![]);
        let results = prove_contracts(&program, &ProverConfig::default());

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, ProofStatus::Assumed);
        assert_eq!(results[1].status, ProofStatus::Proven);
    }
}
