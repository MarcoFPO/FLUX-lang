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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofStatus {
    Proven,
    Disproven,
    Unknown,
    Assumed,
    Timeout,
}

#[derive(Debug, Clone)]
pub struct ProverConfig {
    pub timeout_ms: u32,
}

impl Default for ProverConfig {
    fn default() -> Self {
        Self { timeout_ms: 5000 }
    }
}

// ---------------------------------------------------------------------------
// ProverContext — lookup maps built from the Program AST
// ---------------------------------------------------------------------------

struct ProverContext<'a> {
    consts: HashMap<String, &'a ComputeDef>,
    #[allow(dead_code)] // used in later phases for struct field resolution
    types: HashMap<String, &'a TypeDef>,
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
        ProverContext { consts, types }
    }
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
// prove_clause — negation check pattern
// ---------------------------------------------------------------------------

fn prove_clause<'z>(
    z3_ctx: &'z Context,
    prover_ctx: &ProverContext,
    clause: &ContractClause,
    config: &ProverConfig,
) -> (ProofStatus, Option<String>) {
    match clause {
        ContractClause::Assume { .. } => {
            return (ProofStatus::Assumed, None);
        }
        _ => {}
    }

    let formula = match clause {
        ContractClause::Pre { formula } => formula,
        ContractClause::Post { formula } => formula,
        ContractClause::Invariant { formula } => formula,
        ContractClause::Assume { .. } => unreachable!(),
    };

    let z3_formula = match translate_formula(z3_ctx, prover_ctx, formula) {
        Some(f) => f,
        None => return (ProofStatus::Unknown, Some("untranslatable formula".into())),
    };

    let solver = Solver::new(z3_ctx);
    solver.set_params(&{
        let mut params = z3::Params::new(z3_ctx);
        params.set_u32("timeout", config.timeout_ms);
        params
    });

    // Negation check: assert NOT(formula), check SAT
    // UNSAT → formula always holds → PROVEN
    // SAT → formula can be violated → DISPROVEN
    solver.assert(&z3_formula.not());

    match solver.check() {
        SatResult::Unsat => (ProofStatus::Proven, None),
        SatResult::Sat => {
            let ce = solver
                .get_model()
                .map(|m| format!("{}", m))
                .unwrap_or_default();
            let ce = if ce.is_empty() { None } else { Some(ce) };
            (ProofStatus::Disproven, ce)
        }
        SatResult::Unknown => {
            let reason = solver.get_reason_unknown().unwrap_or_default();
            if reason.contains("timeout") {
                (ProofStatus::Timeout, Some(reason))
            } else {
                (ProofStatus::Unknown, Some(reason))
            }
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

        for (idx, clause) in contract.clauses.iter().enumerate() {
            let kind = match clause {
                ContractClause::Pre { .. } => "pre",
                ContractClause::Post { .. } => "post",
                ContractClause::Invariant { .. } => "invariant",
                ContractClause::Assume { .. } => "assume",
            };

            let (status, counterexample) = if is_extern {
                (ProofStatus::Assumed, None)
            } else {
                prove_clause(&z3_ctx, &prover_ctx, clause, config)
            };

            results.push(ProofResult {
                contract_id: contract.id.as_str().to_string(),
                target_id: contract.target.as_str().to_string(),
                clause_index: idx,
                clause_kind: kind.to_string(),
                status,
                counterexample,
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

    fn make_program(contracts: Vec<ContractDef>, computes: Vec<ComputeDef>) -> Program {
        Program {
            types: vec![],
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
}
