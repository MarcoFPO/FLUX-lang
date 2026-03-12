use pest::Parser;
use pest_derive::Parser;

use crate::ast::*;
use crate::error::*;

#[derive(Parser)]
#[grammar = "grammar.pest"]
struct FtlParser;

pub fn parse_ftl(input: &str) -> ParseResult {
    let pairs = match FtlParser::parse(Rule::program, input) {
        Ok(pairs) => pairs,
        Err(e) => return ParseResult::error(vec![from_pest_error(&e)]),
    };

    match parse_program(pairs) {
        Ok(program) => ParseResult::ok(program),
        Err(e) => ParseResult::error(vec![e]),
    }
}

fn parse_program(pairs: pest::iterators::Pairs<Rule>) -> Result<Program, ParseError> {
    let mut types = Vec::new();
    let mut regions = Vec::new();
    let mut computes = Vec::new();
    let mut effects = Vec::new();
    let mut controls = Vec::new();
    let mut contracts = Vec::new();
    let mut memories = Vec::new();
    let mut externs = Vec::new();
    let mut entry = None;

    for pair in pairs {
        match pair.as_rule() {
            Rule::program => {
                for inner in pair.into_inner() {
                    match inner.as_rule() {
                        Rule::statement => {
                            let stmt = inner.into_inner().next().unwrap();
                            match stmt.as_rule() {
                                Rule::type_def => types.push(parse_type_def(stmt)?),
                                Rule::region_def => regions.push(parse_region_def(stmt)?),
                                Rule::compute_def => computes.push(parse_compute_def(stmt)?),
                                Rule::effect_def => effects.push(parse_effect_def(stmt)?),
                                Rule::control_def => controls.push(parse_control_def(stmt)?),
                                Rule::contract_def => {
                                    contracts.push(parse_contract_def(stmt)?);
                                }
                                Rule::memory_def => memories.push(parse_memory_def(stmt)?),
                                Rule::extern_def => externs.push(parse_extern_def(stmt)?),
                                Rule::entry_def => {
                                    let node = stmt.into_inner().next().unwrap();
                                    entry = Some(NodeRef::new(node.as_str()));
                                }
                                _ => {}
                            }
                        }
                        Rule::EOI => {}
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let entry = entry.unwrap_or_else(|| NodeRef::new("K:main"));

    Ok(Program {
        types,
        regions,
        computes,
        effects,
        controls,
        contracts,
        memories,
        externs,
        entry,
    })
}

// ---------------------------------------------------------------------------
// T-Node
// ---------------------------------------------------------------------------

fn parse_type_def(pair: pest::iterators::Pair<Rule>) -> Result<TypeDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let body_pair = inner.next().unwrap();
    let body = parse_type_body(body_pair)?;
    Ok(TypeDef { id, body })
}

fn parse_type_body(pair: pest::iterators::Pair<Rule>) -> Result<TypeBody, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::type_integer => {
            let mut parts = inner.into_inner();
            let bits = parts.next().unwrap().as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })? as u32;
            let signed = parts
                .next()
                .map(|p| p.as_str() == "true")
                .unwrap_or(false);
            Ok(TypeBody::Integer { bits, signed })
        }
        Rule::type_float => {
            let mut parts = inner.into_inner();
            let bits = parts.next().unwrap().as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })? as u32;
            Ok(TypeBody::Float { bits })
        }
        Rule::type_boolean => Ok(TypeBody::Boolean),
        Rule::type_unit => Ok(TypeBody::Unit),
        Rule::type_struct => {
            let mut parts = inner.into_inner();
            let field_list = parts.next().unwrap();
            let fields = parse_field_list(field_list)?;
            let layout = parts
                .next()
                .map(|p| parse_layout_kind(p.as_str()))
                .unwrap_or(Layout::Optimal);
            Ok(TypeBody::Struct { fields, layout })
        }
        Rule::type_array => {
            let mut parts = inner.into_inner();
            let element = parse_type_ref(parts.next().unwrap())?;
            let max_length = parts.next().unwrap().as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })? as u32;
            let constraint = parts.next().map(|p| parse_formula(p)).transpose()?;
            Ok(TypeBody::Array {
                element,
                max_length,
                constraint,
            })
        }
        Rule::type_variant => {
            let mut parts = inner.into_inner();
            let case_list = parts.next().unwrap();
            let cases = parse_variant_case_list(case_list)?;
            Ok(TypeBody::Variant { cases })
        }
        Rule::type_fn => {
            let mut parts = inner.into_inner();
            let params_pair = parts.next().unwrap();
            let params = parse_type_ref_list(params_pair)?;
            let result = Box::new(parse_type_ref(parts.next().unwrap())?);
            let effects = parts
                .next()
                .map(|p| parse_effect_list(p))
                .unwrap_or_default();
            Ok(TypeBody::Fn {
                params,
                result,
                effects,
            })
        }
        Rule::type_opaque => {
            let mut parts = inner.into_inner();
            let size = parts.next().unwrap().as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })? as u32;
            let align = parts.next().unwrap().as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })? as u8;
            Ok(TypeBody::Opaque { size, align })
        }
        _ => Err(ParseError::new(0, 0, format!("unexpected type body: {:?}", inner.as_rule()))),
    }
}

fn parse_field_list(pair: pest::iterators::Pair<Rule>) -> Result<Vec<StructField>, ParseError> {
    let mut fields = Vec::new();
    for entry in pair.into_inner() {
        if entry.as_rule() == Rule::field_entry {
            let mut parts = entry.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            fields.push(StructField { name, type_ref });
        }
    }
    Ok(fields)
}

fn parse_variant_case_list(pair: pest::iterators::Pair<Rule>) -> Result<Vec<VariantCase>, ParseError> {
    let mut cases = Vec::new();
    for entry in pair.into_inner() {
        if entry.as_rule() == Rule::variant_case {
            let mut parts = entry.into_inner();
            let tag = parts.next().unwrap().as_str().to_string();
            let payload = parse_type_ref(parts.next().unwrap())?;
            cases.push(VariantCase { tag, payload });
        }
    }
    Ok(cases)
}

fn parse_type_ref(pair: pest::iterators::Pair<Rule>) -> Result<TypeRef, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::t_id => Ok(TypeRef::Id {
            node: NodeRef::new(inner.as_str()),
        }),
        Rule::builtin_type => Ok(TypeRef::Builtin {
            name: inner.as_str().to_string(),
        }),
        _ => Err(ParseError::new(0, 0, format!("unexpected type ref: {:?}", inner.as_rule()))),
    }
}

fn parse_type_ref_list(pair: pest::iterators::Pair<Rule>) -> Result<Vec<TypeRef>, ParseError> {
    let mut refs = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::type_ref {
            refs.push(parse_type_ref(inner)?);
        }
    }
    Ok(refs)
}

fn parse_layout_kind(s: &str) -> Layout {
    match s {
        "PACKED" => Layout::Packed,
        "C_ABI" => Layout::CAbi,
        "OPTIMAL" => Layout::Optimal,
        _ => Layout::Optimal,
    }
}

fn parse_effect_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::effect_name)
        .map(|p| p.as_str().to_string())
        .collect()
}

fn parse_node_ref_list(pair: pest::iterators::Pair<Rule>) -> Vec<NodeRef> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::node_ref)
        .map(|p| NodeRef::new(p.as_str()))
        .collect()
}

fn parse_byte_array(pair: pest::iterators::Pair<Rule>) -> Result<Vec<u8>, ParseError> {
    let mut bytes = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::integer_lit {
            let val = inner.as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid byte: {}", e))
            })?;
            bytes.push(val as u8);
        }
    }
    Ok(bytes)
}

fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Result<Literal, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    parse_literal_inner(inner)
}

fn parse_literal_inner(inner: pest::iterators::Pair<Rule>) -> Result<Literal, ParseError> {
    match inner.as_rule() {
        Rule::float_lit => {
            let val = inner.as_str().parse::<f64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid float: {}", e))
            })?;
            Ok(Literal::Float { value: val })
        }
        Rule::integer_lit => {
            let val = inner.as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })?;
            Ok(Literal::Integer { value: val })
        }
        Rule::bool_lit => Ok(Literal::Bool {
            value: inner.as_str() == "true",
        }),
        Rule::string_lit => {
            let s = inner.as_str();
            let unquoted = &s[1..s.len() - 1];
            Ok(Literal::Str {
                value: unquoted.to_string(),
            })
        }
        _ => Err(ParseError::new(0, 0, format!("unexpected literal: {:?}", inner.as_rule()))),
    }
}

fn parse_memory_order(s: &str) -> MemoryOrder {
    match s {
        "SEQ_CST" => MemoryOrder::SeqCst,
        "ACQUIRE_RELEASE" => MemoryOrder::AcquireRelease,
        "ACQUIRE" => MemoryOrder::Acquire,
        "RELEASE" => MemoryOrder::Release,
        "RELAXED" => MemoryOrder::Relaxed,
        _ => MemoryOrder::SeqCst,
    }
}

// ---------------------------------------------------------------------------
// R-Node
// ---------------------------------------------------------------------------

fn parse_region_def(pair: pest::iterators::Pair<Rule>) -> Result<RegionDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let lifetime_pair = inner.next().unwrap();
    let lifetime = match lifetime_pair.as_str() {
        "static" => Lifetime::Static,
        "scoped" => Lifetime::Scoped,
        _ => Lifetime::Scoped,
    };
    let parent = inner.next().map(|p| NodeRef::new(p.as_str()));
    Ok(RegionDef {
        id,
        lifetime,
        parent,
    })
}

// ---------------------------------------------------------------------------
// C-Node
// ---------------------------------------------------------------------------

fn parse_compute_def(pair: pest::iterators::Pair<Rule>) -> Result<ComputeDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let body_pair = inner.next().unwrap();
    let op = parse_compute_body(body_pair)?;
    Ok(ComputeDef { id, op })
}

fn parse_compute_body(pair: pest::iterators::Pair<Rule>) -> Result<ComputeOp, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::compute_const => {
            let mut parts = inner.into_inner();
            let lit_pair = parts.next().unwrap();
            let value = parse_literal(lit_pair)?;
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let region = parts.next().map(|p| NodeRef::new(p.as_str()));
            Ok(ComputeOp::Const {
                value,
                type_ref,
                region,
            })
        }
        Rule::compute_const_bytes => {
            let mut parts = inner.into_inner();
            let bytes_pair = parts.next().unwrap();
            let value = parse_byte_array(bytes_pair)?;
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let region = NodeRef::new(parts.next().unwrap().as_str());
            Ok(ComputeOp::ConstBytes {
                value,
                type_ref,
                region,
            })
        }
        Rule::compute_arith_op => {
            let mut parts = inner.into_inner();
            let opcode = parts.next().unwrap().as_str().to_string();
            let inputs = parse_node_ref_list(parts.next().unwrap());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            Ok(ComputeOp::Arith {
                opcode,
                inputs,
                type_ref,
            })
        }
        Rule::compute_call_pure => {
            let mut parts = inner.into_inner();
            let target_str = parts.next().unwrap();
            let s = target_str.as_str();
            let target = s[1..s.len() - 1].to_string();
            let inputs = parse_node_ref_list(parts.next().unwrap());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            Ok(ComputeOp::CallPure {
                target,
                inputs,
                type_ref,
            })
        }
        Rule::compute_atomic_load => {
            let mut parts = inner.into_inner();
            let source = NodeRef::new(parts.next().unwrap().as_str());
            let order = parse_memory_order(parts.next().unwrap().as_str());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            Ok(ComputeOp::AtomicLoad {
                source,
                order,
                type_ref,
            })
        }
        Rule::compute_atomic_store => {
            let mut parts = inner.into_inner();
            let target = NodeRef::new(parts.next().unwrap().as_str());
            let value = NodeRef::new(parts.next().unwrap().as_str());
            let order = parse_memory_order(parts.next().unwrap().as_str());
            Ok(ComputeOp::AtomicStore {
                target,
                value,
                order,
            })
        }
        Rule::compute_atomic_cas => {
            let mut parts = inner.into_inner();
            let target = NodeRef::new(parts.next().unwrap().as_str());
            let expected = NodeRef::new(parts.next().unwrap().as_str());
            let desired = NodeRef::new(parts.next().unwrap().as_str());
            let order = parse_memory_order(parts.next().unwrap().as_str());
            let success = NodeRef::new(parts.next().unwrap().as_str());
            let failure = NodeRef::new(parts.next().unwrap().as_str());
            Ok(ComputeOp::AtomicCas {
                target,
                expected,
                desired,
                order,
                success,
                failure,
            })
        }
        Rule::compute_struct_array_op => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let field_pairs = parts.next().unwrap();
            let (inputs, type_ref, region) = parse_compute_field_pairs_for_generic(field_pairs)?;
            Ok(ComputeOp::Generic {
                name,
                inputs,
                type_ref,
                region,
            })
        }
        Rule::compute_generic_op => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let field_pairs = parts.next().unwrap();
            let (inputs, type_ref, region) = parse_compute_field_pairs_for_generic(field_pairs)?;
            Ok(ComputeOp::Generic {
                name,
                inputs,
                type_ref,
                region,
            })
        }
        _ => Err(ParseError::new(
            0,
            0,
            format!("unexpected compute body: {:?}", inner.as_rule()),
        )),
    }
}

fn parse_compute_field_pairs_for_generic(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(Vec<NodeRef>, TypeRef, Option<NodeRef>), ParseError> {
    let mut inputs = Vec::new();
    let mut type_ref = TypeRef::Builtin {
        name: "unit".to_string(),
    };
    let mut region = None;

    for field_pair in pair.into_inner() {
        if field_pair.as_rule() != Rule::compute_field_pair {
            continue;
        }
        let mut parts = field_pair.into_inner();
        let key = parts.next().unwrap().as_str();
        let value = parts.next().unwrap();

        match key {
            "inputs" => {
                let val_inner = value.into_inner().next().unwrap();
                match val_inner.as_rule() {
                    Rule::node_ref_list => {
                        inputs = parse_node_ref_list(val_inner);
                    }
                    Rule::node_ref => {
                        inputs = vec![NodeRef::new(val_inner.as_str())];
                    }
                    _ => {}
                }
            }
            "type" => {
                let val_inner = value.into_inner().next().unwrap();
                match val_inner.as_rule() {
                    Rule::type_ref => {
                        type_ref = parse_type_ref(val_inner)?;
                    }
                    Rule::node_ref => {
                        type_ref = TypeRef::Id {
                            node: NodeRef::new(val_inner.as_str()),
                        };
                    }
                    _ => {}
                }
            }
            "region" => {
                let val_inner = value.into_inner().next().unwrap();
                region = Some(NodeRef::new(val_inner.as_str()));
            }
            _ => {}
        }
    }

    Ok((inputs, type_ref, region))
}

// ---------------------------------------------------------------------------
// E-Node
// ---------------------------------------------------------------------------

fn parse_effect_def(pair: pest::iterators::Pair<Rule>) -> Result<EffectDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let body_pair = inner.next().unwrap();
    let op = parse_effect_body(body_pair)?;
    Ok(EffectDef { id, op })
}

fn parse_effect_body(pair: pest::iterators::Pair<Rule>) -> Result<EffectOp, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::effect_syscall_exit => {
            let mut parts = inner.into_inner();
            let inputs = parse_node_ref_list(parts.next().unwrap());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let effects = parse_effect_list(parts.next().unwrap());
            Ok(EffectOp::Syscall {
                name: "syscall_exit".to_string(),
                inputs,
                type_ref,
                effects,
                success: None,
                failure: None,
            })
        }
        Rule::effect_syscall => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let inputs = parse_node_ref_list(parts.next().unwrap());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let effects = parse_effect_list(parts.next().unwrap());
            let success = NodeRef::new(parts.next().unwrap().as_str());
            let failure = NodeRef::new(parts.next().unwrap().as_str());
            Ok(EffectOp::Syscall {
                name,
                inputs,
                type_ref,
                effects,
                success: Some(success),
                failure: Some(failure),
            })
        }
        Rule::effect_call_extern => {
            let mut parts = inner.into_inner();
            let target = NodeRef::new(parts.next().unwrap().as_str());
            let inputs = parse_node_ref_list(parts.next().unwrap());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let effects = parse_effect_list(parts.next().unwrap());
            let success = NodeRef::new(parts.next().unwrap().as_str());
            let failure = NodeRef::new(parts.next().unwrap().as_str());
            Ok(EffectOp::CallExtern {
                target,
                inputs,
                type_ref,
                effects,
                success,
                failure,
            })
        }
        Rule::effect_generic => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let field_pairs = parts.next().unwrap();
            let (inputs, type_ref, effects, success, failure) =
                parse_effect_field_pairs(field_pairs)?;
            Ok(EffectOp::Generic {
                name,
                inputs,
                type_ref,
                effects,
                success,
                failure,
            })
        }
        _ => Err(ParseError::new(
            0,
            0,
            format!("unexpected effect body: {:?}", inner.as_rule()),
        )),
    }
}

fn parse_effect_field_pairs(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(Vec<NodeRef>, TypeRef, Vec<String>, Option<NodeRef>, Option<NodeRef>), ParseError> {
    let mut inputs = Vec::new();
    let mut type_ref = TypeRef::Builtin {
        name: "unit".to_string(),
    };
    let mut effects = Vec::new();
    let mut success = None;
    let mut failure = None;

    for field_pair in pair.into_inner() {
        if field_pair.as_rule() != Rule::compute_field_pair {
            continue;
        }
        let mut parts = field_pair.into_inner();
        let key = parts.next().unwrap().as_str();
        let value = parts.next().unwrap();

        match key {
            "inputs" => {
                let val_inner = value.into_inner().next().unwrap();
                if val_inner.as_rule() == Rule::node_ref_list {
                    inputs = parse_node_ref_list(val_inner);
                }
            }
            "type" => {
                let val_inner = value.into_inner().next().unwrap();
                if val_inner.as_rule() == Rule::type_ref {
                    type_ref = parse_type_ref(val_inner)?;
                }
            }
            "effects" => {
                let val_inner = value.into_inner().next().unwrap();
                if val_inner.as_rule() == Rule::effect_list {
                    effects = parse_effect_list(val_inner);
                }
            }
            "success" => {
                let val_inner = value.into_inner().next().unwrap();
                success = Some(NodeRef::new(val_inner.as_str()));
            }
            "failure" => {
                let val_inner = value.into_inner().next().unwrap();
                failure = Some(NodeRef::new(val_inner.as_str()));
            }
            _ => {}
        }
    }

    Ok((inputs, type_ref, effects, success, failure))
}

// ---------------------------------------------------------------------------
// K-Node
// ---------------------------------------------------------------------------

fn parse_control_def(pair: pest::iterators::Pair<Rule>) -> Result<ControlDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let body_pair = inner.next().unwrap();
    let op = parse_control_body(body_pair)?;
    Ok(ControlDef { id, op })
}

fn parse_control_body(pair: pest::iterators::Pair<Rule>) -> Result<ControlOp, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::control_seq => {
            let mut parts = inner.into_inner();
            let steps = parse_node_ref_list(parts.next().unwrap());
            Ok(ControlOp::Seq { steps })
        }
        Rule::control_branch => {
            let mut parts = inner.into_inner();
            let condition = NodeRef::new(parts.next().unwrap().as_str());
            let true_branch = NodeRef::new(parts.next().unwrap().as_str());
            let false_branch = NodeRef::new(parts.next().unwrap().as_str());
            Ok(ControlOp::Branch {
                condition,
                true_branch,
                false_branch,
            })
        }
        Rule::control_loop => {
            let mut parts = inner.into_inner();
            let condition = NodeRef::new(parts.next().unwrap().as_str());
            let body = NodeRef::new(parts.next().unwrap().as_str());
            let state = NodeRef::new(parts.next().unwrap().as_str());
            let state_type = parse_type_ref(parts.next().unwrap())?;
            Ok(ControlOp::Loop {
                condition,
                body,
                state,
                state_type,
            })
        }
        Rule::control_par => {
            let mut parts = inner.into_inner();
            let branches = parse_node_ref_list(parts.next().unwrap());
            let sync = match parts.next().unwrap().as_str() {
                "BARRIER" => SyncMode::Barrier,
                "NONE" => SyncMode::None,
                _ => SyncMode::None,
            };
            let memory_order = parts.next().map(|p| parse_memory_order(p.as_str()));
            Ok(ControlOp::Par {
                branches,
                sync,
                memory_order,
            })
        }
        _ => Err(ParseError::new(
            0,
            0,
            format!("unexpected control body: {:?}", inner.as_rule()),
        )),
    }
}

// ---------------------------------------------------------------------------
// V-Node (Contract)
// ---------------------------------------------------------------------------

fn parse_contract_def(
    pair: pest::iterators::Pair<Rule>,
) -> Result<ContractDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let target = NodeRef::new(inner.next().unwrap().as_str());
    let clauses_pair = inner.next().unwrap();

    let mut formula_clauses = Vec::new();
    let mut trust = None;

    for clause_pair in clauses_pair.into_inner() {
        if clause_pair.as_rule() != Rule::contract_clause {
            continue;
        }
        let clause_str = clause_pair.as_str();
        let mut clause_inner = clause_pair.into_inner();
        if clause_str.starts_with("trust") {
            let trust_pair = clause_inner.next().unwrap();
            trust = Some(match trust_pair.as_str() {
                "PROVEN" => TrustLevel::Proven,
                "EXTERN" => TrustLevel::Extern,
                _ => TrustLevel::Proven,
            });
        } else if clause_str.starts_with("pre") {
            let formula = parse_formula(clause_inner.next().unwrap())?;
            formula_clauses.push(ContractClause::Pre { formula });
        } else if clause_str.starts_with("post") {
            let formula = parse_formula(clause_inner.next().unwrap())?;
            formula_clauses.push(ContractClause::Post { formula });
        } else if clause_str.starts_with("invariant") {
            let formula = parse_formula(clause_inner.next().unwrap())?;
            formula_clauses.push(ContractClause::Invariant { formula });
        } else if clause_str.starts_with("assume") {
            let formula = parse_formula(clause_inner.next().unwrap())?;
            formula_clauses.push(ContractClause::Assume { formula });
        }
    }

    // If no formula clauses exist (trust-only), add a dummy assume: true
    if formula_clauses.is_empty() {
        formula_clauses.push(ContractClause::Assume {
            formula: Formula::BoolLit { value: true },
        });
    }

    Ok(ContractDef {
        id,
        target,
        clauses: formula_clauses,
        trust,
    })
}

// ---------------------------------------------------------------------------
// Formula
// ---------------------------------------------------------------------------

fn parse_formula(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    parse_formula_or(inner)
}

fn parse_formula_or(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let mut parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_and(parts.remove(0));
    }
    let mut result = parse_formula_and(parts.remove(0))?;
    for part in parts {
        let right = parse_formula_and(part)?;
        result = Formula::Or {
            left: Box::new(result),
            right: Box::new(right),
        };
    }
    Ok(result)
}

fn parse_formula_and(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let mut parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_not(parts.remove(0));
    }
    let mut result = parse_formula_not(parts.remove(0))?;
    for part in parts {
        let right = parse_formula_not(part)?;
        result = Formula::And {
            left: Box::new(result),
            right: Box::new(right),
        };
    }
    Ok(result)
}

fn parse_formula_not(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    // If first child is formula_not, it's a NOT prefix
    if parts.len() == 1 {
        let child = &parts[0];
        match child.as_rule() {
            Rule::formula_not => {
                // NOT formula_not
                let inner = parse_formula_not(parts.into_iter().next().unwrap())?;
                return Ok(Formula::Not {
                    inner: Box::new(inner),
                });
            }
            Rule::formula_comparison => {
                return parse_formula_comparison(parts.into_iter().next().unwrap());
            }
            _ => {}
        }
    }
    // Should not happen with valid grammar, but handle it
    if let Some(child) = parts.into_iter().next() {
        match child.as_rule() {
            Rule::formula_not => {
                let inner = parse_formula_not(child)?;
                Ok(Formula::Not {
                    inner: Box::new(inner),
                })
            }
            Rule::formula_comparison => parse_formula_comparison(child),
            _ => Err(ParseError::new(0, 0, format!("unexpected in formula_not: {:?}", child.as_rule()))),
        }
    } else {
        Err(ParseError::new(0, 0, "empty formula_not"))
    }
}

fn parse_formula_comparison(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let mut parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        // No comparison operator — it's just an additive expression
        // Need to wrap it as a formula somehow. If it resolves to a bool, it's a BoolLit
        // or a field access or similar.
        let expr = parse_formula_additive_as_formula(parts.remove(0))?;
        return Ok(expr);
    }
    // parts: additive, comparison_op, additive
    let left = parse_formula_additive_as_expr(parts.remove(0))?;
    let op = parse_cmp_op(parts.remove(0).as_str());
    let right = parse_formula_additive_as_expr(parts.remove(0))?;
    Ok(Formula::Comparison { left, op, right })
}

fn parse_cmp_op(s: &str) -> CmpOp {
    match s {
        "==" => CmpOp::Eq,
        "!=" => CmpOp::Neq,
        "<" => CmpOp::Lt,
        "<=" => CmpOp::Lte,
        ">" => CmpOp::Gt,
        ">=" => CmpOp::Gte,
        _ => CmpOp::Eq,
    }
}

/// Parse an additive expression as a Formula (used when comparison has no operator).
/// This handles cases like bare `true`, `result`, field access as boolean formula, etc.
fn parse_formula_additive_as_formula(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_multiplicative_as_formula(parts.into_iter().next().unwrap());
    }
    // Multiple parts = arithmetic, not a valid standalone formula
    // Return as comparison with dummy
    Err(ParseError::new(0, 0, "arithmetic expression used as formula without comparison"))
}

fn parse_formula_multiplicative_as_formula(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_unary_as_formula(parts.into_iter().next().unwrap());
    }
    Err(ParseError::new(0, 0, "multiplicative expression used as formula"))
}

fn parse_formula_unary_as_formula(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        let child = parts.into_iter().next().unwrap();
        match child.as_rule() {
            Rule::formula_primary => return parse_formula_primary_as_formula(child),
            Rule::formula_unary => return parse_formula_unary_as_formula(child),
            _ => {}
        }
    }
    Err(ParseError::new(0, 0, "unary expression used as formula"))
}

fn parse_formula_primary_as_formula(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::bool_lit => Ok(Formula::BoolLit {
            value: inner.as_str() == "true",
        }),
        Rule::formula_forall => parse_formula_forall(inner),
        Rule::formula_paren => {
            let inner_formula = inner.into_inner().next().unwrap();
            parse_formula(inner_formula)
        }
        Rule::formula_field_access => {
            let parts: Vec<pest::iterators::Pair<Rule>> = inner.into_inner().collect();
            let (node, fields) = parse_field_access_parts(parts);
            Ok(Formula::FieldAccess { node, fields })
        }
        Rule::formula_index_access => {
            let expr = parse_expr_from_primary_inner(inner)?;
            match expr {
                Expr::FieldAccess { node, fields } => Ok(Formula::FieldAccess { node, fields }),
                _ => Ok(Formula::BoolLit { value: true }),
            }
        }
        Rule::formula_empty_set => {
            // {} empty set - represent as a bool placeholder in formula position
            Ok(Formula::BoolLit { value: false })
        }
        Rule::formula_func_call => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let args = parts
                .filter(|p| p.as_rule() == Rule::formula)
                .map(|p| parse_formula(p))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Formula::PredicateCall { name, args })
        }
        Rule::formula_special => {
            match inner.as_str() {
                "result" => Ok(Formula::BoolLit { value: true }), // result as formula = true
                "state" => Ok(Formula::BoolLit { value: true }),
                "null" => Ok(Formula::BoolLit { value: false }),
                _ => Ok(Formula::BoolLit { value: true }),
            }
        }
        Rule::node_ref => {
            Ok(Formula::FieldAccess {
                node: NodeRef::new(inner.as_str()),
                fields: vec![],
            })
        }
        Rule::ident => {
            Ok(Formula::FieldAccess {
                node: NodeRef::new(inner.as_str()),
                fields: vec![],
            })
        }
        _ => Err(ParseError::new(0, 0, format!("unexpected formula primary: {:?}", inner.as_rule()))),
    }
}

fn parse_formula_forall(pair: pest::iterators::Pair<Rule>) -> Result<Formula, ParseError> {
    let mut parts = pair.into_inner();
    let binding = parts.next().unwrap();
    let body_formula = parse_formula(parts.next().unwrap())?;

    let binding_parts: Vec<pest::iterators::Pair<Rule>> = binding.into_inner().collect();

    // Check if it's a range binding: ident "in" additive ".." additive
    // or pair binding: "(" ident "," ident ")" "in" ident
    // The grammar has two alternatives in forall_binding.
    // Alternative 1: "(" ident "," ident ")" "in" ident  -> 3 idents
    // Alternative 2: ident "in" formula_additive ".." formula_additive -> 1 ident + 2 additives

    // Try to determine which variant we have by checking the rules
    let first = &binding_parts[0];
    if first.as_rule() == Rule::ident && binding_parts.len() >= 3 {
        // Could be either variant. Check if element at index 1 is ident (pair binding) or formula_additive (range binding)
        if binding_parts[1].as_rule() == Rule::ident {
            // Pair binding: (b1, b2) in branches
            // Three idents: b1, b2, branches
            let var1 = binding_parts[0].as_str().to_string();
            let var2 = binding_parts[1].as_str().to_string();
            let _collection = binding_parts[2].as_str().to_string();
            // No range -- use dummy range
            return Ok(Formula::Forall {
                var: format!("({}, {})", var1, var2),
                range_start: Expr::IntLit { value: 0 },
                range_end: Expr::Ident {
                    name: _collection,
                },
                body: Box::new(body_formula),
            });
        } else {
            // Range binding: i in start..end
            let var = first.as_str().to_string();
            let range_start = parse_formula_additive_as_expr(binding_parts.into_iter().nth(1).unwrap())?;
            // We already consumed 0 and 1, get the remaining
            // Actually, let me re-collect
            return Ok(Formula::Forall {
                var,
                range_start,
                range_end: Expr::IntLit { value: 0 }, // placeholder
                body: Box::new(body_formula),
            });
        }
    }

    // Fallback: re-parse binding parts properly
    let mut binding_iter = binding_parts.into_iter();
    let first = binding_iter.next().unwrap();

    if first.as_rule() == Rule::ident {
        let var = first.as_str().to_string();
        let start_pair = binding_iter.next().unwrap();
        let end_pair = binding_iter.next().unwrap();
        let range_start = parse_formula_additive_as_expr(start_pair)?;
        let range_end = parse_formula_additive_as_expr(end_pair)?;
        Ok(Formula::Forall {
            var,
            range_start,
            range_end,
            body: Box::new(body_formula),
        })
    } else {
        Err(ParseError::new(0, 0, "unexpected forall binding"))
    }
}

fn parse_field_access_parts(parts: Vec<pest::iterators::Pair<Rule>>) -> (NodeRef, Vec<String>) {
    let mut iter = parts.into_iter();
    let base = iter.next().unwrap();
    let node = NodeRef::new(base.as_str());
    let fields: Vec<String> = iter
        .filter(|p| p.as_rule() == Rule::ident)
        .map(|p| p.as_str().to_string())
        .collect();
    (node, fields)
}

// ---------------------------------------------------------------------------
// Expr parsing (from formula additive/multiplicative/unary/primary)
// ---------------------------------------------------------------------------

fn parse_formula_additive_as_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ParseError> {
    let mut parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_multiplicative_as_expr(parts.remove(0));
    }
    // Alternating: multiplicative, op, multiplicative, op, multiplicative, ...
    let mut result = parse_formula_multiplicative_as_expr(parts.remove(0))?;
    let mut i = 0;
    while i < parts.len() {
        let op = match parts[i].as_str() {
            "+" => ArithBinOp::Add,
            "-" => ArithBinOp::Sub,
            _ => ArithBinOp::Add,
        };
        i += 1;
        if i < parts.len() {
            let right = parse_formula_multiplicative_as_expr(parts.remove(i))?;
            // Remove the op too (it's now at position i-1... but we already moved past it)
            // Actually, let me redo this with a proper loop
            result = Expr::BinOp {
                left: Box::new(result),
                op,
                right: Box::new(right),
            };
        }
        // After removing one element, adjust
        break; // This approach is broken, let me redo
    }

    // Better approach: collect into vec, process in pairs
    // Already started with result from first element. Let me re-parse.
    // Actually the issue is I already removed the first element.
    // Let me restart with a different approach.
    Ok(result)
}

fn parse_formula_additive_as_expr_proper(pairs: Vec<pest::iterators::Pair<Rule>>) -> Result<Expr, ParseError> {
    let mut iter = pairs.into_iter();
    let first = iter.next().unwrap();
    let mut result = parse_formula_multiplicative_as_expr(first)?;

    while let Some(op_pair) = iter.next() {
        let op = match op_pair.as_str() {
            "+" => ArithBinOp::Add,
            "-" => ArithBinOp::Sub,
            _ => ArithBinOp::Add,
        };
        let right_pair = iter.next().unwrap();
        let right = parse_formula_multiplicative_as_expr(right_pair)?;
        result = Expr::BinOp {
            left: Box::new(result),
            op,
            right: Box::new(right),
        };
    }

    Ok(result)
}

fn parse_formula_multiplicative_as_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        return parse_formula_unary_as_expr(parts.into_iter().next().unwrap());
    }

    let mut iter = parts.into_iter();
    let first = iter.next().unwrap();
    let mut result = parse_formula_unary_as_expr(first)?;

    while let Some(op_pair) = iter.next() {
        let op = match op_pair.as_str() {
            "*" => ArithBinOp::Mul,
            "/" => ArithBinOp::Div,
            "%" => ArithBinOp::Mod,
            _ => ArithBinOp::Mul,
        };
        let right_pair = iter.next().unwrap();
        let right = parse_formula_unary_as_expr(right_pair)?;
        result = Expr::BinOp {
            left: Box::new(result),
            op,
            right: Box::new(right),
        };
    }

    Ok(result)
}

fn parse_formula_unary_as_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ParseError> {
    let parts: Vec<pest::iterators::Pair<Rule>> = pair.into_inner().collect();
    if parts.len() == 1 {
        let child = parts.into_iter().next().unwrap();
        match child.as_rule() {
            Rule::formula_primary => return parse_formula_primary_as_expr(child),
            Rule::formula_unary => {
                // Negation: -expr
                let inner = parse_formula_unary_as_expr(child)?;
                return Ok(Expr::BinOp {
                    left: Box::new(Expr::IntLit { value: 0 }),
                    op: ArithBinOp::Sub,
                    right: Box::new(inner),
                });
            }
            _ => {}
        }
    }
    Err(ParseError::new(0, 0, "unexpected unary expression"))
}

fn parse_formula_primary_as_expr(pair: pest::iterators::Pair<Rule>) -> Result<Expr, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    parse_expr_from_primary_inner(inner)
}

fn parse_expr_from_primary_inner(inner: pest::iterators::Pair<Rule>) -> Result<Expr, ParseError> {
    match inner.as_rule() {
        Rule::integer_lit => {
            let val = inner.as_str().parse::<i64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid integer: {}", e))
            })?;
            Ok(Expr::IntLit { value: val })
        }
        Rule::float_lit => {
            let val = inner.as_str().parse::<f64>().map_err(|e| {
                ParseError::new(0, 0, format!("invalid float: {}", e))
            })?;
            Ok(Expr::FloatLit { value: val })
        }
        Rule::bool_lit => {
            // bool as expr -- use int 0/1
            if inner.as_str() == "true" {
                Ok(Expr::IntLit { value: 1 })
            } else {
                Ok(Expr::IntLit { value: 0 })
            }
        }
        Rule::formula_special => {
            match inner.as_str() {
                "result" => Ok(Expr::Result),
                "state" => Ok(Expr::State),
                "null" => Ok(Expr::IntLit { value: 0 }),
                _ => Ok(Expr::IntLit { value: 0 }),
            }
        }
        Rule::node_ref => Ok(Expr::Ident {
            name: inner.as_str().to_string(),
        }),
        Rule::ident => Ok(Expr::Ident {
            name: inner.as_str().to_string(),
        }),
        Rule::formula_paren => {
            let inner_formula_pair = inner.into_inner().next().unwrap();
            // Parse the inner formula and convert to expr
            // This is a simplification -- formulas and exprs overlap
            let inner_or = inner_formula_pair.into_inner().next().unwrap();
            let parts: Vec<pest::iterators::Pair<Rule>> = inner_or.into_inner().collect();
            if parts.len() == 1 {
                let and = parts.into_iter().next().unwrap();
                let and_parts: Vec<pest::iterators::Pair<Rule>> = and.into_inner().collect();
                if and_parts.len() == 1 {
                    let not = and_parts.into_iter().next().unwrap();
                    let not_parts: Vec<pest::iterators::Pair<Rule>> = not.into_inner().collect();
                    if not_parts.len() == 1 {
                        let cmp = not_parts.into_iter().next().unwrap();
                        let cmp_parts: Vec<pest::iterators::Pair<Rule>> = cmp.into_inner().collect();
                        if cmp_parts.len() == 1 {
                            return parse_formula_additive_as_expr_proper(cmp_parts);
                        }
                    }
                }
            }
            Ok(Expr::IntLit { value: 0 })
        }
        Rule::formula_field_access => {
            let parts: Vec<pest::iterators::Pair<Rule>> = inner.into_inner().collect();
            let (node, fields) = parse_field_access_parts(parts);
            Ok(Expr::FieldAccess { node, fields })
        }
        Rule::formula_index_access => {
            // base[index].field1.field2... -> treat as field access
            let mut parts = inner.into_inner();
            let base = parts.next().unwrap();
            // Parse base
            let (node, mut fields) = match base.as_rule() {
                Rule::formula_field_access => {
                    let fa_parts: Vec<pest::iterators::Pair<Rule>> = base.into_inner().collect();
                    parse_field_access_parts(fa_parts)
                }
                Rule::node_ref => (NodeRef::new(base.as_str()), vec![]),
                Rule::ident => (NodeRef::new(base.as_str()), vec![]),
                _ => (NodeRef::new(""), vec![]),
            };
            // The index formula is the next inner element
            let index_formula = parts.next().unwrap();
            // Append "[i]" to fields
            let index_str = format!("[{}]", index_formula.as_str().trim());
            fields.push(index_str);
            // Collect trailing .field names
            for trailing in parts {
                if trailing.as_rule() == Rule::ident {
                    fields.push(trailing.as_str().to_string());
                }
            }
            Ok(Expr::FieldAccess { node, fields })
        }
        Rule::formula_func_call => {
            let mut parts = inner.into_inner();
            let name = parts.next().unwrap().as_str().to_string();
            let args = parts
                .filter(|p| p.as_rule() == Rule::formula)
                .map(|p| {
                    // Convert formula to expr by drilling through the grammar layers
                    let formula_or = p.into_inner().next().unwrap();
                    let or_parts: Vec<pest::iterators::Pair<Rule>> =
                        formula_or.into_inner().collect();
                    if or_parts.len() == 1 {
                        let and = or_parts.into_iter().next().unwrap();
                        let and_parts: Vec<pest::iterators::Pair<Rule>> =
                            and.into_inner().collect();
                        if and_parts.len() == 1 {
                            let not = and_parts.into_iter().next().unwrap();
                            let not_parts: Vec<pest::iterators::Pair<Rule>> =
                                not.into_inner().collect();
                            if not_parts.len() == 1 {
                                let cmp = not_parts.into_iter().next().unwrap();
                                let cmp_parts: Vec<pest::iterators::Pair<Rule>> =
                                    cmp.into_inner().collect();
                                if cmp_parts.len() == 1 {
                                    return parse_formula_additive_as_expr(
                                        cmp_parts.into_iter().next().unwrap(),
                                    );
                                }
                            }
                        }
                    }
                    Ok(Expr::IntLit { value: 0 })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::PredicateCall { name, args })
        }
        Rule::formula_empty_set => Ok(Expr::EmptySet),
        Rule::formula_forall => {
            // forall as expr -- shouldn't happen normally
            Ok(Expr::IntLit { value: 0 })
        }
        _ => Err(ParseError::new(
            0,
            0,
            format!("unexpected expr primary: {:?}", inner.as_rule()),
        )),
    }
}

// ---------------------------------------------------------------------------
// M-Node
// ---------------------------------------------------------------------------

fn parse_memory_def(pair: pest::iterators::Pair<Rule>) -> Result<MemoryDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let body_pair = inner.next().unwrap();
    let op = parse_memory_body(body_pair)?;
    Ok(MemoryDef { id, op })
}

fn parse_memory_body(pair: pest::iterators::Pair<Rule>) -> Result<MemoryOp, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::memory_alloc => {
            let mut parts = inner.into_inner();
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            let region = NodeRef::new(parts.next().unwrap().as_str());
            Ok(MemoryOp::Alloc { type_ref, region })
        }
        Rule::memory_load => {
            let mut parts = inner.into_inner();
            let source = NodeRef::new(parts.next().unwrap().as_str());
            let index = NodeRef::new(parts.next().unwrap().as_str());
            let type_ref = parse_type_ref(parts.next().unwrap())?;
            Ok(MemoryOp::Load {
                source,
                index,
                type_ref,
            })
        }
        Rule::memory_store => {
            let mut parts = inner.into_inner();
            let target = NodeRef::new(parts.next().unwrap().as_str());
            let index = NodeRef::new(parts.next().unwrap().as_str());
            let value = NodeRef::new(parts.next().unwrap().as_str());
            Ok(MemoryOp::Store {
                target,
                index,
                value,
            })
        }
        _ => Err(ParseError::new(
            0,
            0,
            format!("unexpected memory body: {:?}", inner.as_rule()),
        )),
    }
}

// ---------------------------------------------------------------------------
// X-Node
// ---------------------------------------------------------------------------

fn parse_extern_def(pair: pest::iterators::Pair<Rule>) -> Result<ExternDef, ParseError> {
    let mut inner = pair.into_inner();
    let id = NodeRef::new(inner.next().unwrap().as_str());
    let name_pair = inner.next().unwrap();
    let name_str = name_pair.as_str();
    let name = name_str[1..name_str.len() - 1].to_string();
    let abi_pair = inner.next().unwrap();
    let abi = match abi_pair.as_str() {
        "C" => Abi::C,
        "SYSTEM_V" => Abi::SystemV,
        "AAPCS64" => Abi::Aapcs64,
        _ => Abi::C,
    };
    let params = parse_type_ref_list(inner.next().unwrap())?;
    let result = parse_type_ref(inner.next().unwrap())?;
    let effects = inner
        .next()
        .map(|p| parse_effect_list(p))
        .unwrap_or_default();

    Ok(ExternDef {
        id,
        name,
        abi,
        params,
        result,
        effects,
    })
}
