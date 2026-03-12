use std::collections::HashMap;

use serde::Serialize;

use crate::ast::*;

// ---------------------------------------------------------------------------
// CheckError — unified error type for type and effect checks
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CheckError {
    pub error_code: u32,
    pub node_id: String,
    pub violation: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Type index — maps type node IDs and builtin names to their TypeBody
// ---------------------------------------------------------------------------

fn build_type_index(program: &Program) -> HashMap<String, TypeBody> {
    let mut index = HashMap::new();

    // Register all user-defined T-Nodes
    for td in &program.types {
        index.insert(td.id.as_str().to_string(), td.body.clone());
    }

    // Synthetic builtin types
    let builtins: Vec<(&str, TypeBody)> = vec![
        ("u8", TypeBody::Integer { bits: 8, signed: false }),
        ("u16", TypeBody::Integer { bits: 16, signed: false }),
        ("u32", TypeBody::Integer { bits: 32, signed: false }),
        ("u64", TypeBody::Integer { bits: 64, signed: false }),
        ("i8", TypeBody::Integer { bits: 8, signed: true }),
        ("i16", TypeBody::Integer { bits: 16, signed: true }),
        ("i32", TypeBody::Integer { bits: 32, signed: true }),
        ("i64", TypeBody::Integer { bits: 64, signed: true }),
        ("f32", TypeBody::Float { bits: 32 }),
        ("f64", TypeBody::Float { bits: 64 }),
        ("bool", TypeBody::Boolean),
        ("boolean", TypeBody::Boolean),
        ("unit", TypeBody::Unit),
    ];

    for (name, body) in builtins {
        index.entry(name.to_string()).or_insert(body);
    }

    index
}

// ---------------------------------------------------------------------------
// Resolve a TypeRef to a TypeBody (if available in the index)
// ---------------------------------------------------------------------------

fn resolve_type<'a>(tr: &TypeRef, index: &'a HashMap<String, TypeBody>) -> Option<&'a TypeBody> {
    match tr {
        TypeRef::Id { node } => index.get(node.as_str()),
        TypeRef::Builtin { name } => index.get(name.as_str()),
    }
}

/// Return a human-readable label for a TypeRef.
fn type_ref_label(tr: &TypeRef, index: &HashMap<String, TypeBody>) -> String {
    match tr {
        TypeRef::Id { node } => {
            if let Some(body) = index.get(node.as_str()) {
                format!("{} ({})", node, type_body_short(body))
            } else {
                node.to_string()
            }
        }
        TypeRef::Builtin { name } => name.clone(),
    }
}

fn type_ref_key(tr: &TypeRef) -> String {
    match tr {
        TypeRef::Id { node } => node.as_str().to_string(),
        TypeRef::Builtin { name } => name.clone(),
    }
}

fn type_body_short(body: &TypeBody) -> &'static str {
    match body {
        TypeBody::Integer { signed: true, bits: 8 } => "i8",
        TypeBody::Integer { signed: true, bits: 16 } => "i16",
        TypeBody::Integer { signed: true, bits: 32 } => "i32",
        TypeBody::Integer { signed: true, bits: 64 } => "i64",
        TypeBody::Integer { signed: false, bits: 8 } => "u8",
        TypeBody::Integer { signed: false, bits: 16 } => "u16",
        TypeBody::Integer { signed: false, bits: 32 } => "u32",
        TypeBody::Integer { signed: false, bits: 64 } => "u64",
        TypeBody::Integer { .. } => "integer",
        TypeBody::Float { bits: 32 } => "f32",
        TypeBody::Float { bits: 64 } => "f64",
        TypeBody::Float { .. } => "float",
        TypeBody::Boolean => "bool",
        TypeBody::Unit => "unit",
        TypeBody::Struct { .. } => "struct",
        TypeBody::Array { .. } => "array",
        TypeBody::Variant { .. } => "variant",
        TypeBody::Fn { .. } => "fn",
        TypeBody::Opaque { .. } => "opaque",
    }
}

fn is_integer_type(body: &TypeBody) -> bool {
    matches!(body, TypeBody::Integer { .. })
}

fn is_float_type(body: &TypeBody) -> bool {
    matches!(body, TypeBody::Float { .. })
}

fn is_numeric_type(body: &TypeBody) -> bool {
    is_integer_type(body) || is_float_type(body)
}

/// Returns true if the given node ID is a constant with integer value 0.
/// A zero literal is a valid zero-initializer for any type.
fn is_zero_literal(node_id: &str, program: &Program) -> bool {
    program.computes.iter().any(|c| {
        c.id.as_str() == node_id
            && matches!(
                &c.op,
                ComputeOp::Const {
                    value: Literal::Integer { value: 0 },
                    ..
                }
            )
    })
}

// ---------------------------------------------------------------------------
// Build a lookup from NodeRef → TypeRef for compute nodes
// ---------------------------------------------------------------------------

fn build_compute_type_map(program: &Program) -> HashMap<String, TypeRef> {
    let mut map = HashMap::new();
    for c in &program.computes {
        let tr = match &c.op {
            ComputeOp::Const { type_ref, .. }
            | ComputeOp::ConstBytes { type_ref, .. }
            | ComputeOp::Arith { type_ref, .. }
            | ComputeOp::CallPure { type_ref, .. }
            | ComputeOp::Generic { type_ref, .. }
            | ComputeOp::AtomicLoad { type_ref, .. } => Some(type_ref),
            ComputeOp::AtomicStore { .. } | ComputeOp::AtomicCas { .. } => None,
        };
        if let Some(tr) = tr {
            map.insert(c.id.as_str().to_string(), tr.clone());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Build an alloc-type map: M-Node alloc ID → TypeRef
// ---------------------------------------------------------------------------

fn build_alloc_type_map(program: &Program) -> HashMap<String, TypeRef> {
    let mut map = HashMap::new();
    for m in &program.memories {
        if let MemoryOp::Alloc { type_ref, .. } = &m.op {
            map.insert(m.id.as_str().to_string(), type_ref.clone());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Build extern index: X-Node ID → &ExternDef
// ---------------------------------------------------------------------------

fn build_extern_index(program: &Program) -> HashMap<String, &ExternDef> {
    program.externs.iter().map(|x| (x.id.as_str().to_string(), x)).collect()
}

// ---------------------------------------------------------------------------
// Check: types match (structural equality by key)
// ---------------------------------------------------------------------------

fn types_match(a: &TypeRef, b: &TypeRef) -> bool {
    type_ref_key(a) == type_ref_key(b)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn check_types_and_effects(program: &Program) -> Vec<CheckError> {
    let mut errors = Vec::new();
    let type_index = build_type_index(program);

    check_compute_types(program, &type_index, &mut errors);
    check_effect_nodes(program, &mut errors);
    check_control_nodes(program, &type_index, &mut errors);
    check_memory_types(program, &type_index, &mut errors);
    check_extern_types(program, &type_index, &mut errors);

    errors
}

// ---------------------------------------------------------------------------
// 3xxx — Compute type checks
// ---------------------------------------------------------------------------

fn check_compute_types(
    program: &Program,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    let compute_type_map = build_compute_type_map(program);

    for c in &program.computes {
        let node_id = c.id.as_str().to_string();

        match &c.op {
            // --- Const: value must be compatible with declared type ---
            ComputeOp::Const { value, type_ref, .. } => {
                if let Some(body) = resolve_type(type_ref, type_index) {
                    let ok = match (value, body) {
                        // Integer 0/1 can represent boolean values
                        (Literal::Integer { value: 0 | 1 }, TypeBody::Boolean) => true,
                        // Integer 0 is valid as zero-initialization for any composite type
                        (Literal::Integer { value: 0 }, _) => true,
                        (Literal::Integer { .. }, tb) => is_integer_type(tb),
                        (Literal::Float { .. }, tb) => is_float_type(tb),
                        (Literal::Bool { .. }, TypeBody::Boolean) => true,
                        (Literal::Str { .. }, TypeBody::Array { .. }) => true,
                        (Literal::Str { .. }, TypeBody::Opaque { .. }) => true,
                        _ => false,
                    };
                    if !ok {
                        errors.push(CheckError {
                            error_code: 3001,
                            node_id: node_id.clone(),
                            violation: "TYPE_MISMATCH".into(),
                            message: format!(
                                "Const value {:?} is not compatible with declared type {}",
                                value,
                                type_ref_label(type_ref, type_index),
                            ),
                            suggestion: Some(format!(
                                "Change the type of {} to match the literal kind, or change the literal value",
                                node_id,
                            )),
                        });
                    }
                }
            }

            // --- Arith: all inputs must share the output type ---
            ComputeOp::Arith { inputs, type_ref, opcode, .. } => {
                let is_comparison_op = matches!(
                    opcode.as_str(),
                    "eq" | "ne" | "lt" | "le" | "gt" | "ge"
                );

                if is_comparison_op {
                    // Comparison ops: output must be Boolean, inputs must be
                    // compatible with each other (but NOT with the output type).
                    if let Some(body) = resolve_type(type_ref, type_index) {
                        if !matches!(body, TypeBody::Boolean) {
                            errors.push(CheckError {
                                error_code: 3001,
                                node_id: node_id.clone(),
                                violation: "TYPE_MISMATCH".into(),
                                message: format!(
                                    "Comparison op '{}' must return Boolean, but got {}",
                                    opcode,
                                    type_ref_label(type_ref, type_index),
                                ),
                                suggestion: Some(format!(
                                    "Change the output type of {} to Boolean",
                                    node_id,
                                )),
                            });
                        }
                    }
                    // Check inputs are compatible with each other
                    let input_types: Vec<_> = inputs.iter()
                        .filter_map(|i| compute_type_map.get(i.as_str()))
                        .collect();
                    if input_types.len() >= 2 {
                        let first = input_types[0];
                        for (idx, tr) in input_types[1..].iter().enumerate() {
                            if !types_match(first, tr) {
                                // Allow compatible integer types
                                let compatible = matches!(
                                    (resolve_type(first, type_index), resolve_type(tr, type_index)),
                                    (Some(a), Some(b)) if is_integer_type(a) && is_integer_type(b)
                                );
                                if !compatible {
                                    errors.push(CheckError {
                                        error_code: 3001,
                                        node_id: node_id.clone(),
                                        violation: "TYPE_MISMATCH".into(),
                                        message: format!(
                                            "Comparison op '{}' inputs must have compatible types, but input {} has type {} vs {}",
                                            opcode,
                                            inputs[idx + 1],
                                            type_ref_label(tr, type_index),
                                            type_ref_label(first, type_index),
                                        ),
                                        suggestion: Some(format!(
                                            "Ensure all inputs to {} have compatible types",
                                            node_id,
                                        )),
                                    });
                                }
                            }
                        }
                    }
                } else {
                    // Non-comparison arith ops: check inputs match output type
                    for input in inputs {
                        if let Some(input_tr) = compute_type_map.get(input.as_str()) {
                            if !types_match(input_tr, type_ref) {
                                // Allow compatible integer types (different widths)
                                let compatible = matches!(
                                    (resolve_type(input_tr, type_index), resolve_type(type_ref, type_index)),
                                    (Some(a), Some(b)) if is_integer_type(a) && is_integer_type(b)
                                );
                                if !compatible {
                                    errors.push(CheckError {
                                        error_code: 3001,
                                        node_id: node_id.clone(),
                                        violation: "TYPE_MISMATCH".into(),
                                        message: format!(
                                            "Arith op '{}' expects all inputs of type {}, but input {} has type {}",
                                            opcode,
                                            type_ref_label(type_ref, type_index),
                                            input,
                                            type_ref_label(input_tr, type_index),
                                        ),
                                        suggestion: Some(format!(
                                            "Ensure all inputs to {} have type {}",
                                            node_id,
                                            type_ref_label(type_ref, type_index),
                                        )),
                                    });
                                }
                            }
                        }
                    }
                    // Also check that arith operates on compatible types
                    if let Some(body) = resolve_type(type_ref, type_index) {
                        let is_boolean_op = matches!(opcode.as_str(), "and" | "or" | "not" | "xor");
                        let type_ok = is_numeric_type(body)
                            || (is_boolean_op && matches!(body, TypeBody::Boolean));
                        if !type_ok {
                            errors.push(CheckError {
                                error_code: 3001,
                                node_id: node_id.clone(),
                                violation: "TYPE_MISMATCH".into(),
                                message: format!(
                                    "Arith op '{}' requires a numeric or boolean type, but got {}",
                                    opcode,
                                    type_ref_label(type_ref, type_index),
                                ),
                                suggestion: Some(format!(
                                    "Change the output type of {} to an integer, float, or boolean type",
                                    node_id,
                                )),
                            });
                        }
                    }
                }
            }

            // --- CallPure: parameter count / types vs fn-type signature ---
            ComputeOp::CallPure { target, inputs, type_ref, .. } => {
                // Look up target in type index (it may refer to a T-Node with Fn body)
                if let Some(TypeBody::Fn { params, result, .. }) = type_index.get(target.as_str()) {
                    // Check param count
                    if inputs.len() != params.len() {
                        errors.push(CheckError {
                            error_code: 3005,
                            node_id: node_id.clone(),
                            violation: "FN_PARAM_MISMATCH".into(),
                            message: format!(
                                "call_pure to '{}' expects {} parameters, but got {}",
                                target,
                                params.len(),
                                inputs.len(),
                            ),
                            suggestion: Some(format!(
                                "Adjust the number of inputs to {} to match the function signature",
                                node_id,
                            )),
                        });
                    } else {
                        // Check each param type
                        for (i, (input, param_tr)) in inputs.iter().zip(params.iter()).enumerate() {
                            if let Some(input_tr) = compute_type_map.get(input.as_str()) {
                                if !types_match(input_tr, param_tr) {
                                    errors.push(CheckError {
                                        error_code: 3005,
                                        node_id: node_id.clone(),
                                        violation: "FN_PARAM_MISMATCH".into(),
                                        message: format!(
                                            "call_pure to '{}': parameter {} expects type {}, but input {} has type {}",
                                            target,
                                            i,
                                            type_ref_label(param_tr, type_index),
                                            input,
                                            type_ref_label(input_tr, type_index),
                                        ),
                                        suggestion: None,
                                    });
                                }
                            }
                        }
                    }
                    // Check result type matches declared type_ref
                    if !types_match(result, type_ref) {
                        errors.push(CheckError {
                            error_code: 3001,
                            node_id: node_id.clone(),
                            violation: "TYPE_MISMATCH".into(),
                            message: format!(
                                "call_pure to '{}' returns {}, but {} declares type {}",
                                target,
                                type_ref_label(result, type_index),
                                node_id,
                                type_ref_label(type_ref, type_index),
                            ),
                            suggestion: Some(format!(
                                "Change the declared type of {} to match the function's return type",
                                node_id,
                            )),
                        });
                    }
                }
            }

            // --- Generic: check struct_get field existence ---
            ComputeOp::Generic { name, inputs, type_ref, .. } => {
                if name == "struct_get" || name == "struct_set" {
                    check_struct_field_access(
                        &node_id,
                        name,
                        inputs,
                        type_ref,
                        &compute_type_map,
                        type_index,
                        errors,
                    );
                }
                if name == "array_get" || name == "array_load" {
                    check_array_index(
                        &node_id,
                        name,
                        inputs,
                        &compute_type_map,
                        type_index,
                        errors,
                    );
                }
                if name == "variant_tag" {
                    // variant_tag should produce an integer
                    if let Some(body) = resolve_type(type_ref, type_index) {
                        if !is_integer_type(body) {
                            errors.push(CheckError {
                                error_code: 3001,
                                node_id: node_id.clone(),
                                violation: "TYPE_MISMATCH".into(),
                                message: format!(
                                    "variant_tag should produce an integer type, but {} has type {}",
                                    node_id,
                                    type_ref_label(type_ref, type_index),
                                ),
                                suggestion: Some("variant_tag result type should be an integer (e.g. u16)".into()),
                            });
                        }
                    }
                }
            }

            // AtomicLoad, AtomicStore, AtomicCas — no extra type checks beyond what
            // the memory checks already cover
            _ => {}
        }
    }
}

/// Check struct_get / struct_set: verify the referenced field exists in the struct type.
fn check_struct_field_access(
    node_id: &str,
    op_name: &str,
    inputs: &[NodeRef],
    _type_ref: &TypeRef,
    compute_type_map: &HashMap<String, TypeRef>,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    // Convention: struct_get inputs = [struct_value, field_name_const]
    // We check the struct input's type for the field.
    // The field name is typically encoded in the second input as a const string,
    // but since we don't have a direct way to extract it from the graph at this
    // level, we do a best-effort check: verify the first input is a struct type.
    if let Some(first_input) = inputs.first() {
        if let Some(input_tr) = compute_type_map.get(first_input.as_str()) {
            if let Some(body) = resolve_type(input_tr, type_index) {
                if !matches!(body, TypeBody::Struct { .. }) {
                    errors.push(CheckError {
                        error_code: 3002,
                        node_id: node_id.to_string(),
                        violation: "INVALID_FIELD".into(),
                        message: format!(
                            "{} on {} expects a struct type, but {} has type {}",
                            op_name,
                            node_id,
                            first_input,
                            type_ref_label(input_tr, type_index),
                        ),
                        suggestion: Some(format!(
                            "Ensure the first input to {} is a struct-typed node",
                            node_id,
                        )),
                    });
                }
            }
        }
    }
}

/// Check that array index input is an integer type.
fn check_array_index(
    node_id: &str,
    op_name: &str,
    inputs: &[NodeRef],
    compute_type_map: &HashMap<String, TypeRef>,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    // Convention: array_get inputs = [array, index]
    // The index (second input) must be integer.
    if inputs.len() >= 2 {
        let index_ref = &inputs[1];
        if let Some(index_tr) = compute_type_map.get(index_ref.as_str()) {
            if let Some(body) = resolve_type(index_tr, type_index) {
                if !is_integer_type(body) {
                    errors.push(CheckError {
                        error_code: 3004,
                        node_id: node_id.to_string(),
                        violation: "INVALID_ARRAY_INDEX".into(),
                        message: format!(
                            "{} on {} expects an integer index, but {} has type {}",
                            op_name,
                            node_id,
                            index_ref,
                            type_ref_label(index_tr, type_index),
                        ),
                        suggestion: Some(format!(
                            "Change the index input of {} to an integer-typed node",
                            node_id,
                        )),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 4xxx — Effect node checks
// ---------------------------------------------------------------------------

fn check_effect_nodes(program: &Program, errors: &mut Vec<CheckError>) {
    // Collect all node IDs for reachability checks
    let all_ids: std::collections::HashSet<String> = collect_all_node_ids(program);

    for e in &program.effects {
        let node_id = e.id.as_str().to_string();

        match &e.op {
            EffectOp::Syscall {
                name,
                success,
                failure,
                effects,
                ..
            } => {
                let is_exit = name == "syscall_exit" || name == "exit";

                if is_exit {
                    // 4004: syscall_exit should not have success/failure
                    if success.is_some() || failure.is_some() {
                        errors.push(CheckError {
                            error_code: 4004,
                            node_id: node_id.clone(),
                            violation: "EXIT_WITH_SUCCESS_FAILURE".into(),
                            message: format!(
                                "{} is a terminating syscall but defines success/failure paths",
                                node_id,
                            ),
                            suggestion: Some(format!(
                                "Remove success/failure from {} — syscall_exit terminates the process",
                                node_id,
                            )),
                        });
                    }
                } else {
                    // 4001: non-exit syscall must have both success AND failure
                    if success.is_none() || failure.is_none() {
                        let missing = match (success.is_none(), failure.is_none()) {
                            (true, true) => "success and failure",
                            (true, false) => "success",
                            (false, true) => "failure",
                            _ => unreachable!(),
                        };
                        errors.push(CheckError {
                            error_code: 4001,
                            node_id: node_id.clone(),
                            violation: "MISSING_FAILURE_PATH".into(),
                            message: format!(
                                "{} (syscall '{}') is missing {} path(s)",
                                node_id, name, missing,
                            ),
                            suggestion: Some(format!(
                                "Add {} path(s) to {} — every E-Node must handle both outcomes",
                                missing, node_id,
                            )),
                        });
                    }

                    // 4002: check that success/failure targets exist
                    if let Some(s) = success {
                        if !all_ids.contains(s.as_str()) {
                            errors.push(CheckError {
                                error_code: 4002,
                                node_id: node_id.clone(),
                                violation: "UNREACHABLE_PATH".into(),
                                message: format!(
                                    "{} success path references non-existent node {}",
                                    node_id, s,
                                ),
                                suggestion: Some(format!(
                                    "Define node {} or update the success path of {}",
                                    s, node_id,
                                )),
                            });
                        }
                    }
                    if let Some(f) = failure {
                        if !all_ids.contains(f.as_str()) {
                            errors.push(CheckError {
                                error_code: 4002,
                                node_id: node_id.clone(),
                                violation: "UNREACHABLE_PATH".into(),
                                message: format!(
                                    "{} failure path references non-existent node {}",
                                    node_id, f,
                                ),
                                suggestion: Some(format!(
                                    "Define node {} or update the failure path of {}",
                                    f, node_id,
                                )),
                            });
                        }
                    }
                }

                // 4003: empty effects list (warning)
                if effects.is_empty() {
                    errors.push(CheckError {
                        error_code: 4003,
                        node_id: node_id.clone(),
                        violation: "MISSING_EFFECT_DECLARATION".into(),
                        message: format!(
                            "{} (syscall '{}') has an empty effects list",
                            node_id, name,
                        ),
                        suggestion: Some(format!(
                            "Declare at least one effect (IO, PROC, NET, MEM) for {}",
                            node_id,
                        )),
                    });
                }
            }

            EffectOp::CallExtern {
                success,
                failure,
                effects,
                target,
                ..
            } => {
                // CallExtern always requires both paths (fields are non-optional)
                // but check reachability
                if !all_ids.contains(success.as_str()) {
                    errors.push(CheckError {
                        error_code: 4002,
                        node_id: node_id.clone(),
                        violation: "UNREACHABLE_PATH".into(),
                        message: format!(
                            "{} success path references non-existent node {}",
                            node_id, success,
                        ),
                        suggestion: Some(format!(
                            "Define node {} or update the success path of {}",
                            success, node_id,
                        )),
                    });
                }
                if !all_ids.contains(failure.as_str()) {
                    errors.push(CheckError {
                        error_code: 4002,
                        node_id: node_id.clone(),
                        violation: "UNREACHABLE_PATH".into(),
                        message: format!(
                            "{} failure path references non-existent node {}",
                            node_id, failure,
                        ),
                        suggestion: Some(format!(
                            "Define node {} or update the failure path of {}",
                            failure, node_id,
                        )),
                    });
                }

                // 4003: empty effects
                if effects.is_empty() {
                    errors.push(CheckError {
                        error_code: 4003,
                        node_id: node_id.clone(),
                        violation: "MISSING_EFFECT_DECLARATION".into(),
                        message: format!(
                            "{} (call_extern to {}) has an empty effects list",
                            node_id, target,
                        ),
                        suggestion: Some(format!(
                            "Declare at least one effect for {}",
                            node_id,
                        )),
                    });
                }
            }

            EffectOp::Generic {
                name,
                success,
                failure,
                effects,
                ..
            } => {
                // 4001: generic E-Nodes should also have both paths
                if success.is_none() || failure.is_none() {
                    let missing = match (success.is_none(), failure.is_none()) {
                        (true, true) => "success and failure",
                        (true, false) => "success",
                        (false, true) => "failure",
                        _ => unreachable!(),
                    };
                    errors.push(CheckError {
                        error_code: 4001,
                        node_id: node_id.clone(),
                        violation: "MISSING_FAILURE_PATH".into(),
                        message: format!(
                            "{} (effect '{}') is missing {} path(s)",
                            node_id, name, missing,
                        ),
                        suggestion: Some(format!(
                            "Add {} path(s) to {}",
                            missing, node_id,
                        )),
                    });
                }

                // 4002: reachability
                if let Some(s) = success {
                    if !all_ids.contains(s.as_str()) {
                        errors.push(CheckError {
                            error_code: 4002,
                            node_id: node_id.clone(),
                            violation: "UNREACHABLE_PATH".into(),
                            message: format!(
                                "{} success path references non-existent node {}",
                                node_id, s,
                            ),
                            suggestion: Some(format!("Define node {} or update the success path of {}", s, node_id)),
                        });
                    }
                }
                if let Some(f) = failure {
                    if !all_ids.contains(f.as_str()) {
                        errors.push(CheckError {
                            error_code: 4002,
                            node_id: node_id.clone(),
                            violation: "UNREACHABLE_PATH".into(),
                            message: format!(
                                "{} failure path references non-existent node {}",
                                node_id, f,
                            ),
                            suggestion: Some(format!("Define node {} or update the failure path of {}", f, node_id)),
                        });
                    }
                }

                // 4003: empty effects
                if effects.is_empty() {
                    errors.push(CheckError {
                        error_code: 4003,
                        node_id: node_id.clone(),
                        violation: "MISSING_EFFECT_DECLARATION".into(),
                        message: format!(
                            "{} (effect '{}') has an empty effects list",
                            node_id, name,
                        ),
                        suggestion: Some(format!("Declare at least one effect for {}", node_id)),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3xxx + 5xxx — Control node checks
// ---------------------------------------------------------------------------

fn check_control_nodes(
    program: &Program,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    let compute_type_map = build_compute_type_map(program);

    for k in &program.controls {
        let node_id = k.id.as_str().to_string();

        match &k.op {
            // K:Loop — state_type must resolve to a known T-Node
            ControlOp::Loop { state_type, condition, .. } => {
                if resolve_type(state_type, type_index).is_none() {
                    errors.push(CheckError {
                        error_code: 3001,
                        node_id: node_id.clone(),
                        violation: "TYPE_MISMATCH".into(),
                        message: format!(
                            "Loop {} declares state_type {} which is not a defined type",
                            node_id,
                            type_ref_label(state_type, type_index),
                        ),
                        suggestion: Some(format!(
                            "Define the type {} or use an existing type for the loop state",
                            type_ref_label(state_type, type_index),
                        )),
                    });
                }

                // condition should be boolean-typed
                if let Some(cond_tr) = compute_type_map.get(condition.as_str()) {
                    if let Some(body) = resolve_type(cond_tr, type_index) {
                        if !matches!(body, TypeBody::Boolean) {
                            errors.push(CheckError {
                                error_code: 3001,
                                node_id: node_id.clone(),
                                violation: "TYPE_MISMATCH".into(),
                                message: format!(
                                    "Loop {} condition {} should be boolean, but has type {}",
                                    node_id,
                                    condition,
                                    type_ref_label(cond_tr, type_index),
                                ),
                                suggestion: Some("Loop conditions must evaluate to a boolean".into()),
                            });
                        }
                    }
                }
            }

            // K:Branch — condition should be boolean; check exhaustiveness for variant
            ControlOp::Branch { condition, .. } => {
                if let Some(cond_tr) = compute_type_map.get(condition.as_str()) {
                    if let Some(body) = resolve_type(cond_tr, type_index) {
                        // If condition is a variant_tag result, we'd need to check
                        // exhaustiveness — but a simple Branch only has true/false,
                        // so variant exhaustiveness applies to multi-way branches.
                        // For a binary branch on boolean, this is fine.
                        // For a binary branch on a variant with >2 cases, it's non-exhaustive.
                        if let TypeBody::Variant { cases } = body {
                            if cases.len() > 2 {
                                errors.push(CheckError {
                                    error_code: 3003,
                                    node_id: node_id.clone(),
                                    violation: "NON_EXHAUSTIVE_MATCH".into(),
                                    message: format!(
                                        "Branch {} on variant type with {} cases uses only true/false branching — not all cases are covered",
                                        node_id,
                                        cases.len(),
                                    ),
                                    suggestion: Some(format!(
                                        "Use a multi-way branch or nested branches to cover all {} variant cases",
                                        cases.len(),
                                    )),
                                });
                            }
                        }
                    }
                }
            }

            // K:Par — check branch count
            ControlOp::Par { branches, .. } => {
                if branches.len() < 2 {
                    errors.push(CheckError {
                        error_code: 5001,
                        node_id: node_id.clone(),
                        violation: "PAR_SINGLE_BRANCH".into(),
                        message: format!(
                            "Par {} has only {} branch(es) — parallel execution requires at least 2",
                            node_id,
                            branches.len(),
                        ),
                        suggestion: Some(format!(
                            "Add more branches to {} or use K:Seq instead",
                            node_id,
                        )),
                    });
                }
            }

            ControlOp::Seq { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 3xxx — Memory type checks
// ---------------------------------------------------------------------------

fn check_memory_types(
    program: &Program,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    let alloc_type_map = build_alloc_type_map(program);
    let compute_type_map = build_compute_type_map(program);

    for m in &program.memories {
        let node_id = m.id.as_str().to_string();

        match &m.op {
            MemoryOp::Load { source, type_ref, index, .. } => {
                // Check that load type matches the alloc type or its element type
                if let Some(alloc_tr) = alloc_type_map.get(source.as_str()) {
                    let direct_match = types_match(alloc_tr, type_ref);
                    let element_match = match resolve_type(alloc_tr, type_index) {
                        Some(TypeBody::Array { element, .. }) => types_match(&element, type_ref),
                        Some(TypeBody::Struct { .. }) => true, // struct field access
                        _ => false,
                    };
                    if !direct_match && !element_match {
                        errors.push(CheckError {
                            error_code: 3001,
                            node_id: node_id.clone(),
                            violation: "TYPE_MISMATCH".into(),
                            message: format!(
                                "Load {} declares type {} but source {} was allocated as {}",
                                node_id,
                                type_ref_label(type_ref, type_index),
                                source,
                                type_ref_label(alloc_tr, type_index),
                            ),
                            suggestion: Some(format!(
                                "Change the type of {} to match the allocation or element type of {}",
                                node_id, source,
                            )),
                        });
                    }
                }
                // Check that index is integer-typed
                if let Some(index_tr) = compute_type_map.get(index.as_str()) {
                    if let Some(body) = resolve_type(index_tr, type_index) {
                        if !is_integer_type(body) {
                            errors.push(CheckError {
                                error_code: 3004,
                                node_id: node_id.clone(),
                                violation: "INVALID_ARRAY_INDEX".into(),
                                message: format!(
                                    "Load {} index {} must be an integer, but has type {}",
                                    node_id,
                                    index,
                                    type_ref_label(index_tr, type_index),
                                ),
                                suggestion: Some(format!(
                                    "Change the index of {} to an integer-typed node",
                                    node_id,
                                )),
                            });
                        }
                    }
                }
            }

            MemoryOp::Store { target, value, index, .. } => {
                // Check that stored value type matches the alloc type or its element type
                if let Some(alloc_tr) = alloc_type_map.get(target.as_str()) {
                    if let Some(value_tr) = compute_type_map.get(value.as_str()) {
                        let direct_match = types_match(value_tr, alloc_tr);
                        let element_match = match resolve_type(alloc_tr, type_index) {
                            Some(TypeBody::Array { element, .. }) => {
                                types_match(value_tr, &element)
                                || is_zero_literal(value.as_str(), program)
                            }
                            Some(TypeBody::Struct { .. }) => true, // struct field write
                            _ => false,
                        };
                        if !direct_match && !element_match {
                            errors.push(CheckError {
                                error_code: 3001,
                                node_id: node_id.clone(),
                                violation: "TYPE_MISMATCH".into(),
                                message: format!(
                                    "Store {} writes value {} of type {} to {} which was allocated as {}",
                                    node_id,
                                    value,
                                    type_ref_label(value_tr, type_index),
                                    target,
                                    type_ref_label(alloc_tr, type_index),
                                ),
                                suggestion: Some(format!(
                                    "Ensure the value type matches the allocation or element type of {}",
                                    target,
                                )),
                            });
                        }
                    }
                }
                // Check that index is integer-typed
                if let Some(index_tr) = compute_type_map.get(index.as_str()) {
                    if let Some(body) = resolve_type(index_tr, type_index) {
                        if !is_integer_type(body) {
                            errors.push(CheckError {
                                error_code: 3004,
                                node_id: node_id.clone(),
                                violation: "INVALID_ARRAY_INDEX".into(),
                                message: format!(
                                    "Store {} index {} must be an integer, but has type {}",
                                    node_id,
                                    index,
                                    type_ref_label(index_tr, type_index),
                                ),
                                suggestion: Some(format!(
                                    "Change the index of {} to an integer-typed node",
                                    node_id,
                                )),
                            });
                        }
                    }
                }
            }

            MemoryOp::Alloc { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 3xxx — Extern type checks (FFI parameter/result validation)
// ---------------------------------------------------------------------------

fn check_extern_types(
    program: &Program,
    type_index: &HashMap<String, TypeBody>,
    errors: &mut Vec<CheckError>,
) {
    let compute_type_map = build_compute_type_map(program);
    let extern_index = build_extern_index(program);

    // Check call_extern E-Nodes against their X-Node declaration
    for e in &program.effects {
        if let EffectOp::CallExtern {
            target,
            inputs,
            type_ref,
            ..
        } = &e.op
        {
            let node_id = e.id.as_str().to_string();

            if let Some(ext) = extern_index.get(target.as_str()) {
                // Check parameter count
                if inputs.len() != ext.params.len() {
                    errors.push(CheckError {
                        error_code: 3005,
                        node_id: node_id.clone(),
                        violation: "FN_PARAM_MISMATCH".into(),
                        message: format!(
                            "call_extern {} to '{}' expects {} parameters, but got {}",
                            node_id,
                            ext.name,
                            ext.params.len(),
                            inputs.len(),
                        ),
                        suggestion: Some(format!(
                            "Adjust the number of inputs to {} to match the extern declaration {}",
                            node_id, target,
                        )),
                    });
                } else {
                    // Check each parameter type
                    for (i, (input, param_tr)) in inputs.iter().zip(ext.params.iter()).enumerate() {
                        if let Some(input_tr) = compute_type_map.get(input.as_str()) {
                            if !types_match(input_tr, param_tr) {
                                errors.push(CheckError {
                                    error_code: 3005,
                                    node_id: node_id.clone(),
                                    violation: "FN_PARAM_MISMATCH".into(),
                                    message: format!(
                                        "call_extern {} to '{}': parameter {} expects type {}, but input {} has type {}",
                                        node_id,
                                        ext.name,
                                        i,
                                        type_ref_label(param_tr, type_index),
                                        input,
                                        type_ref_label(input_tr, type_index),
                                    ),
                                    suggestion: None,
                                });
                            }
                        }
                    }
                }

                // Check result type
                if !types_match(&ext.result, type_ref) {
                    errors.push(CheckError {
                        error_code: 3001,
                        node_id: node_id.clone(),
                        violation: "TYPE_MISMATCH".into(),
                        message: format!(
                            "call_extern {} declares type {} but extern '{}' returns {}",
                            node_id,
                            type_ref_label(type_ref, type_index),
                            ext.name,
                            type_ref_label(&ext.result, type_index),
                        ),
                        suggestion: Some(format!(
                            "Change the declared type of {} to match the extern return type",
                            node_id,
                        )),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Utility: collect all node IDs in the program
// ---------------------------------------------------------------------------

fn collect_all_node_ids(program: &Program) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for t in &program.types {
        ids.insert(t.id.as_str().to_string());
    }
    for r in &program.regions {
        ids.insert(r.id.as_str().to_string());
    }
    for c in &program.computes {
        ids.insert(c.id.as_str().to_string());
    }
    for e in &program.effects {
        ids.insert(e.id.as_str().to_string());
    }
    for k in &program.controls {
        ids.insert(k.id.as_str().to_string());
    }
    for v in &program.contracts {
        ids.insert(v.id.as_str().to_string());
    }
    for m in &program.memories {
        ids.insert(m.id.as_str().to_string());
    }
    for x in &program.externs {
        ids.insert(x.id.as_str().to_string());
    }
    ids
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: minimal valid program with no nodes.
    fn empty_program() -> Program {
        Program {
            types: vec![],
            regions: vec![],
            computes: vec![],
            effects: vec![],
            controls: vec![],
            contracts: vec![],
            memories: vec![],
            externs: vec![],
            entry: NodeRef::new("K:entry"),
        }
    }

    #[test]
    fn test_empty_program_no_errors() {
        let errors = check_types_and_effects(&empty_program());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_const_type_mismatch() {
        let mut p = empty_program();
        p.types.push(TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 64, signed: false },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Bool { value: true },
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
                region: None,
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 3001));
    }

    #[test]
    fn test_const_type_ok() {
        let mut p = empty_program();
        p.types.push(TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 64, signed: false },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 42 },
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
                region: None,
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_missing_failure_path() {
        let mut p = empty_program();
        p.effects.push(EffectDef {
            id: NodeRef::new("E:d1"),
            op: EffectOp::Syscall {
                name: "syscall_write".into(),
                inputs: vec![],
                type_ref: TypeRef::Builtin { name: "unit".into() },
                effects: vec!["IO".into()],
                success: Some(NodeRef::new("K:f1")),
                failure: None,
            },
        });
        // Add K:f1 so success is reachable
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f1"),
            op: ControlOp::Seq { steps: vec![] },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 4001));
    }

    #[test]
    fn test_exit_with_success_failure_warning() {
        let mut p = empty_program();
        p.effects.push(EffectDef {
            id: NodeRef::new("E:d1"),
            op: EffectOp::Syscall {
                name: "syscall_exit".into(),
                inputs: vec![],
                type_ref: TypeRef::Builtin { name: "unit".into() },
                effects: vec!["PROC".into()],
                success: Some(NodeRef::new("K:f1")),
                failure: None,
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 4004));
    }

    #[test]
    fn test_empty_effects_warning() {
        let mut p = empty_program();
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f1"),
            op: ControlOp::Seq { steps: vec![] },
        });
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f2"),
            op: ControlOp::Seq { steps: vec![] },
        });
        p.effects.push(EffectDef {
            id: NodeRef::new("E:d1"),
            op: EffectOp::Syscall {
                name: "syscall_write".into(),
                inputs: vec![],
                type_ref: TypeRef::Builtin { name: "unit".into() },
                effects: vec![],
                success: Some(NodeRef::new("K:f1")),
                failure: Some(NodeRef::new("K:f2")),
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 4003));
    }

    #[test]
    fn test_par_single_branch_warning() {
        let mut p = empty_program();
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f1"),
            op: ControlOp::Par {
                branches: vec![NodeRef::new("K:f2")],
                sync: SyncMode::Barrier,
                memory_order: None,
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 5001));
    }

    #[test]
    fn test_unreachable_path() {
        let mut p = empty_program();
        p.effects.push(EffectDef {
            id: NodeRef::new("E:d1"),
            op: EffectOp::Syscall {
                name: "syscall_read".into(),
                inputs: vec![],
                type_ref: TypeRef::Builtin { name: "unit".into() },
                effects: vec!["IO".into()],
                success: Some(NodeRef::new("K:nonexistent1")),
                failure: Some(NodeRef::new("K:nonexistent2")),
            },
        });
        let errors = check_types_and_effects(&p);
        let unreachable_errors: Vec<_> = errors.iter().filter(|e| e.error_code == 4002).collect();
        assert_eq!(unreachable_errors.len(), 2);
    }

    #[test]
    fn test_arith_type_mismatch() {
        let mut p = empty_program();
        p.types.push(TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 64, signed: false },
        });
        p.types.push(TypeDef {
            id: NodeRef::new("T:a2"),
            body: TypeBody::Float { bits: 64 },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 1 },
                type_ref: TypeRef::Id { node: NodeRef::new("T:a2") },
                region: None,
            },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Arith {
                opcode: "add".into(),
                inputs: vec![NodeRef::new("C:c1")],
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 3001));
    }

    #[test]
    fn test_memory_store_type_mismatch() {
        let mut p = empty_program();
        p.types.push(TypeDef {
            id: NodeRef::new("T:a1"),
            body: TypeBody::Integer { bits: 64, signed: false },
        });
        p.types.push(TypeDef {
            id: NodeRef::new("T:a2"),
            body: TypeBody::Boolean,
        });
        p.regions.push(RegionDef {
            id: NodeRef::new("R:b1"),
            lifetime: Lifetime::Scoped,
            parent: None,
        });
        p.memories.push(MemoryDef {
            id: NodeRef::new("M:m1"),
            op: MemoryOp::Alloc {
                type_ref: TypeRef::Id { node: NodeRef::new("T:a1") },
                region: NodeRef::new("R:b1"),
            },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:val"),
            op: ComputeOp::Const {
                value: Literal::Bool { value: true },
                type_ref: TypeRef::Id { node: NodeRef::new("T:a2") },
                region: None,
            },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:idx"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 0 },
                type_ref: TypeRef::Builtin { name: "u64".into() },
                region: None,
            },
        });
        p.memories.push(MemoryDef {
            id: NodeRef::new("M:m2"),
            op: MemoryOp::Store {
                target: NodeRef::new("M:m1"),
                index: NodeRef::new("C:idx"),
                value: NodeRef::new("C:val"),
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 3001 && e.node_id == "M:m2"));
    }

    #[test]
    fn test_extern_param_mismatch() {
        let mut p = empty_program();
        p.types.push(TypeDef {
            id: NodeRef::new("T:ptr"),
            body: TypeBody::Integer { bits: 64, signed: false },
        });
        p.externs.push(ExternDef {
            id: NodeRef::new("X:ext1"),
            name: "memcpy".into(),
            abi: Abi::C,
            params: vec![
                TypeRef::Id { node: NodeRef::new("T:ptr") },
                TypeRef::Id { node: NodeRef::new("T:ptr") },
            ],
            result: TypeRef::Id { node: NodeRef::new("T:ptr") },
            effects: vec!["MEM".into()],
        });
        // call_extern with wrong number of params
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f1"),
            op: ControlOp::Seq { steps: vec![] },
        });
        p.controls.push(ControlDef {
            id: NodeRef::new("K:f2"),
            op: ControlOp::Seq { steps: vec![] },
        });
        p.computes.push(ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 0 },
                type_ref: TypeRef::Id { node: NodeRef::new("T:ptr") },
                region: None,
            },
        });
        p.effects.push(EffectDef {
            id: NodeRef::new("E:d1"),
            op: EffectOp::CallExtern {
                target: NodeRef::new("X:ext1"),
                inputs: vec![NodeRef::new("C:c1")], // only 1 param, expects 2
                type_ref: TypeRef::Id { node: NodeRef::new("T:ptr") },
                effects: vec!["MEM".into()],
                success: NodeRef::new("K:f1"),
                failure: NodeRef::new("K:f2"),
            },
        });
        let errors = check_types_and_effects(&p);
        assert!(errors.iter().any(|e| e.error_code == 3005));
    }
}
