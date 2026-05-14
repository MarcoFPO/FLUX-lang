// ---------------------------------------------------------------------------
// codegen_c.rs — C code generation from FTL AST
// ---------------------------------------------------------------------------
//
// Translates a parsed FTL Program into valid C11 source code.
// This provides a lightweight alternative to the LLVM codegen path:
//   FTL -> C code -> gcc/cc
//
// Supports:
//   - T-Nodes: Integer, Float, Boolean, Unit, Struct, Array, Variant, Fn, Opaque
//   - C-Nodes: Const, ConstBytes, Arith, CallPure, Generic, StructGet/Set,
//              VariantCreate/Is/Get
//   - E-Nodes: Syscall (write, exit, read, open, close, ioctl), CallExtern
//   - K-Nodes: Seq, Branch, Loop, Par
//   - M-Nodes: Alloc, Load, Store
//   - X-Nodes: Extern declarations
//   - F-Nodes: User-defined pure functions
//   - entry -> main()
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::fmt::Write;

use crate::ast::*;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CodegenCError {
    UnresolvedNode(String),
    Unsupported(String),
    FormatError(std::fmt::Error),
}

impl std::fmt::Display for CodegenCError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenCError::UnresolvedNode(msg) => write!(f, "unresolved node: {msg}"),
            CodegenCError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            CodegenCError::FormatError(e) => write!(f, "format error: {e}"),
        }
    }
}

impl std::error::Error for CodegenCError {}

impl From<std::fmt::Error> for CodegenCError {
    fn from(e: std::fmt::Error) -> Self {
        CodegenCError::FormatError(e)
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate C11 source code from a parsed FTL program.
pub fn codegen_c(program: &Program) -> Result<String, CodegenCError> {
    let mut generator = CCodeGenerator::new(program);
    generator.emit_program()
}

// ---------------------------------------------------------------------------
// Internal code generator
// ---------------------------------------------------------------------------

struct CCodeGenerator<'a> {
    program: &'a Program,

    // Lookup tables
    compute_map: HashMap<String, &'a ComputeDef>,
    effect_map: HashMap<String, &'a EffectDef>,
    control_map: HashMap<String, &'a ControlDef>,
    memory_map: HashMap<String, &'a MemoryDef>,
    type_map: HashMap<String, &'a TypeDef>,
    extern_map: HashMap<String, &'a ExternDef>,

    // Track which K-nodes have been emitted (cycle prevention)
    emitted_controls: HashMap<String, bool>,

    // Track which C-nodes have been emitted as statements
    emitted_values: HashMap<String, bool>,
}

impl<'a> CCodeGenerator<'a> {
    fn new(program: &'a Program) -> Self {
        let mut compute_map = HashMap::new();
        for c in &program.computes {
            compute_map.insert(c.id.0.clone(), c);
        }
        let mut effect_map = HashMap::new();
        for e in &program.effects {
            effect_map.insert(e.id.0.clone(), e);
        }
        let mut control_map = HashMap::new();
        for k in &program.controls {
            control_map.insert(k.id.0.clone(), k);
        }
        let mut memory_map = HashMap::new();
        for m in &program.memories {
            memory_map.insert(m.id.0.clone(), m);
        }
        let mut type_map = HashMap::new();
        for t in &program.types {
            type_map.insert(t.id.0.clone(), t);
        }
        let mut extern_map = HashMap::new();
        for x in &program.externs {
            extern_map.insert(x.id.0.clone(), x);
        }

        Self {
            program,
            compute_map,
            effect_map,
            control_map,
            memory_map,
            type_map,
            extern_map,
            emitted_controls: HashMap::new(),
            emitted_values: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Identifier sanitization
    // ------------------------------------------------------------------

    fn sanitize_id(id: &str) -> String {
        id.replace(':', "_")
    }

    fn is_c_reserved(name: &str) -> bool {
        matches!(
            name,
            "auto" | "break" | "case" | "char" | "const" | "continue"
            | "default" | "do" | "double" | "else" | "enum" | "extern"
            | "float" | "for" | "goto" | "if" | "inline" | "int" | "long"
            | "register" | "restrict" | "return" | "short" | "signed"
            | "sizeof" | "static" | "struct" | "switch" | "typedef"
            | "union" | "unsigned" | "void" | "volatile" | "while"
            | "_Bool" | "_Complex" | "_Imaginary"
            | "bool" | "true" | "false"
            | "main" | "read" | "write" | "open" | "close" | "exit"
            | "malloc" | "free" | "calloc" | "realloc"
            | "printf" | "fprintf" | "sprintf" | "snprintf"
            | "memcpy" | "memset" | "memmove" | "strlen" | "strcmp"
        )
    }

    fn sanitize_fn_name(name: &str) -> String {
        if Self::is_c_reserved(name) {
            format!("ftl_{}", name)
        } else {
            name.to_string()
        }
    }

    // ------------------------------------------------------------------
    // Type conversions
    // ------------------------------------------------------------------

    fn type_ref_to_c(&self, type_ref: &TypeRef) -> String {
        match type_ref {
            TypeRef::Builtin { name } => Self::builtin_type_to_c(name),
            TypeRef::Id { node } => {
                if let Some(td) = self.type_map.get(&node.0) {
                    self.type_body_to_c_name(&td.body, &node.0)
                } else {
                    format!("/* unknown type {} */ int64_t", node.0)
                }
            }
        }
    }

    fn builtin_type_to_c(name: &str) -> String {
        match name {
            "unit" | "void" => "void".to_string(),
            "bool" | "boolean" => "bool".to_string(),
            "u8" => "uint8_t".to_string(),
            "i8" => "int8_t".to_string(),
            "u16" => "uint16_t".to_string(),
            "i16" => "int16_t".to_string(),
            "u32" => "uint32_t".to_string(),
            "i32" => "int32_t".to_string(),
            "u64" => "uint64_t".to_string(),
            "i64" => "int64_t".to_string(),
            "f32" => "float".to_string(),
            "f64" => "double".to_string(),
            _ => "int64_t".to_string(),
        }
    }

    fn type_body_to_c_name(&self, body: &TypeBody, node_id: &str) -> String {
        match body {
            TypeBody::Integer { bits, signed } => match (bits, signed) {
                (8, true) => "int8_t".to_string(),
                (8, false) => "uint8_t".to_string(),
                (16, true) => "int16_t".to_string(),
                (16, false) => "uint16_t".to_string(),
                (32, true) => "int32_t".to_string(),
                (32, false) => "uint32_t".to_string(),
                (64, true) => "int64_t".to_string(),
                (64, false) => "uint64_t".to_string(),
                (1, _) => "bool".to_string(),
                _ => format!("int{}_t", bits),
            },
            TypeBody::Float { bits } => match bits {
                32 => "float".to_string(),
                _ => "double".to_string(),
            },
            TypeBody::Boolean => "bool".to_string(),
            TypeBody::Unit => "void".to_string(),
            TypeBody::Struct { .. } | TypeBody::Variant { .. } => {
                format!("{}_t", Self::sanitize_id(node_id))
            }
            TypeBody::Array { .. } => {
                format!("{}_t", Self::sanitize_id(node_id))
            }
            TypeBody::Fn { .. } => "void*".to_string(),
            TypeBody::Opaque { .. } => "void*".to_string(),
        }
    }

    fn is_void_type(&self, type_ref: &TypeRef) -> bool {
        match type_ref {
            TypeRef::Builtin { name } => name == "unit" || name == "void",
            TypeRef::Id { node } => {
                if let Some(td) = self.type_map.get(&node.0) {
                    matches!(td.body, TypeBody::Unit)
                } else {
                    false
                }
            }
        }
    }

    fn type_ref_byte_size(&self, type_ref: &TypeRef) -> u64 {
        match type_ref {
            TypeRef::Builtin { name } => match name.as_str() {
                "bool" | "boolean" | "u8" | "i8" => 1,
                "u16" | "i16" => 2,
                "u32" | "i32" | "f32" => 4,
                "u64" | "i64" | "f64" => 8,
                _ => 8,
            },
            TypeRef::Id { node } => {
                if let Some(td) = self.type_map.get(&node.0) {
                    match &td.body {
                        TypeBody::Integer { bits, .. } | TypeBody::Float { bits } => {
                            (*bits as u64).div_ceil(8)
                        }
                        TypeBody::Boolean => 1,
                        TypeBody::Unit => 0,
                        TypeBody::Opaque { size, .. } => *size as u64,
                        TypeBody::Array { element, max_length, .. } => {
                            *max_length as u64 * self.type_ref_byte_size(element)
                        }
                        TypeBody::Struct { fields, .. } => {
                            fields.iter().map(|f| self.type_ref_byte_size(&f.type_ref)).sum()
                        }
                        TypeBody::Variant { cases } => {
                            4 + cases.iter().map(|c| self.type_ref_byte_size(&c.payload)).max().unwrap_or(0)
                        }
                        TypeBody::Fn { .. } => 8,
                    }
                } else {
                    8
                }
            }
        }
    }

    fn get_memory_element_size(&self, mem_node_id: &str) -> u64 {
        for mem in &self.program.memories {
            if mem.id.0 == mem_node_id {
                if let MemoryOp::Alloc { type_ref, .. } = &mem.op {
                    if let TypeRef::Id { node } = type_ref {
                        if let Some(td) = self.type_map.get(&node.0) {
                            if let TypeBody::Array { element, .. } = &td.body {
                                return self.type_ref_byte_size(element);
                            }
                        }
                    }
                    return self.type_ref_byte_size(type_ref);
                }
            }
        }
        8
    }

    // ------------------------------------------------------------------
    // Top-level emission
    // ------------------------------------------------------------------

    fn emit_program(&mut self) -> Result<String, CodegenCError> {
        let mut out = String::new();

        writeln!(out, "/* Generated C code from FTL */")?;
        writeln!(out, "#include <stdint.h>")?;
        writeln!(out, "#include <stdbool.h>")?;
        writeln!(out, "#include <unistd.h>")?;
        writeln!(out, "#include <stdlib.h>")?;
        writeln!(out, "#include <string.h>")?;
        writeln!(out, "#include <fcntl.h>")?;
        writeln!(out, "#include <sys/ioctl.h>")?;
        writeln!(out)?;

        self.emit_type_defs(&mut out)?;
        self.emit_extern_decls(&mut out)?;
        self.emit_global_constants(&mut out)?;
        self.emit_fn_defs(&mut out)?;
        self.emit_main(&mut out)?;

        Ok(out)
    }

    // ------------------------------------------------------------------
    // T-Node emission
    // ------------------------------------------------------------------

    fn emit_type_defs(&self, out: &mut String) -> Result<(), CodegenCError> {
        for td in &self.program.types {
            match &td.body {
                TypeBody::Integer { .. } | TypeBody::Float { .. } | TypeBody::Boolean | TypeBody::Unit => {
                    let c_type = self.type_body_to_c_name(&td.body, &td.id.0);
                    if c_type != "void" {
                        writeln!(out, "typedef {} {}_t;", c_type, Self::sanitize_id(&td.id.0))?;
                    }
                }
                TypeBody::Struct { fields, .. } => {
                    let name = Self::sanitize_id(&td.id.0);
                    writeln!(out, "typedef struct {{")?;
                    for field in fields {
                        if let TypeRef::Id { node } = &field.type_ref {
                            if let Some(ftd) = self.type_map.get(&node.0) {
                                if let TypeBody::Array { element, max_length, .. } = &ftd.body {
                                    let elem = self.type_ref_to_c(element);
                                    writeln!(out, "    {} {}[{}];", elem, field.name, max_length)?;
                                    continue;
                                }
                            }
                        }
                        let ftype = self.type_ref_to_c(&field.type_ref);
                        writeln!(out, "    {} {};", ftype, field.name)?;
                    }
                    writeln!(out, "}} {}_t;", name)?;
                    writeln!(out)?;
                }
                TypeBody::Variant { cases } => {
                    let name = Self::sanitize_id(&td.id.0);
                    writeln!(out, "enum {}_tag {{", name)?;
                    for (i, case) in cases.iter().enumerate() {
                        writeln!(out, "    {}_{} = {},", name.to_uppercase(), case.tag.to_uppercase(), i)?;
                    }
                    writeln!(out, "}};")?;
                    writeln!(out, "typedef struct {{")?;
                    writeln!(out, "    int32_t tag;")?;
                    let has_payload = cases.iter().any(|c| !self.is_void_type(&c.payload));
                    if has_payload {
                        writeln!(out, "    union {{")?;
                        for case in cases {
                            if !self.is_void_type(&case.payload) {
                                let ptype = self.type_ref_to_c(&case.payload);
                                writeln!(out, "        {} {};", ptype, case.tag)?;
                            }
                        }
                        writeln!(out, "    }} payload;")?;
                    }
                    writeln!(out, "}} {}_t;", name)?;
                    writeln!(out)?;
                }
                TypeBody::Array { element, max_length, .. } => {
                    let elem = self.type_ref_to_c(element);
                    writeln!(out, "typedef {} {}_t[{}];", elem, Self::sanitize_id(&td.id.0), max_length)?;
                }
                TypeBody::Fn { .. } => {
                    writeln!(out, "typedef void* {}_t;", Self::sanitize_id(&td.id.0))?;
                }
                TypeBody::Opaque { .. } => {
                    writeln!(out, "typedef void* {}_t;", Self::sanitize_id(&td.id.0))?;
                }
            }
        }
        if !self.program.types.is_empty() {
            writeln!(out)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // X-Node extern declarations
    // ------------------------------------------------------------------

    fn emit_extern_decls(&self, out: &mut String) -> Result<(), CodegenCError> {
        for ext in &self.program.externs {
            let ret = self.type_ref_to_c(&ext.result);
            let params: Vec<String> = ext.params.iter().enumerate()
                .map(|(i, p)| format!("{} p{}", self.type_ref_to_c(p), i))
                .collect();
            let ps = if params.is_empty() { "void".to_string() } else { params.join(", ") };
            writeln!(out, "extern {} {}({});", ret, ext.name, ps)?;
        }
        if !self.program.externs.is_empty() {
            writeln!(out)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Global constants (ConstBytes)
    // ------------------------------------------------------------------

    fn emit_global_constants(&self, out: &mut String) -> Result<(), CodegenCError> {
        let mut any = false;
        for compute in &self.program.computes {
            if let ComputeOp::ConstBytes { value, .. } = &compute.op {
                any = true;
                let name = Self::sanitize_id(&compute.id.0);
                write!(out, "static const uint8_t const_{}[] = {{", name)?;
                for (i, b) in value.iter().enumerate() {
                    if i > 0 { write!(out, ",")?; }
                    write!(out, "{}", b)?;
                }
                writeln!(out, "}};")?;
            }
        }
        if any { writeln!(out)?; }
        Ok(())
    }

    // ------------------------------------------------------------------
    // F-Node function definitions
    // ------------------------------------------------------------------

    fn emit_fn_defs(&mut self, out: &mut String) -> Result<(), CodegenCError> {
        // Forward declarations
        for fn_def in &self.program.functions {
            let raw = fn_def.id.as_str().strip_prefix("F:").unwrap_or(fn_def.id.as_str());
            let fn_name = Self::sanitize_fn_name(raw);
            let ret = self.type_ref_to_c(&fn_def.result);
            let params: Vec<String> = fn_def.params.iter()
                .map(|p| format!("{} {}", self.type_ref_to_c(&p.type_ref), p.name))
                .collect();
            let ps = if params.is_empty() { "void".to_string() } else { params.join(", ") };
            writeln!(out, "{} {}({});", ret, fn_name, ps)?;
        }
        if !self.program.functions.is_empty() { writeln!(out)?; }

        // Bodies
        for fn_def in &self.program.functions {
            let raw = fn_def.id.as_str().strip_prefix("F:").unwrap_or(fn_def.id.as_str());
            let fn_name = Self::sanitize_fn_name(raw);
            let ret = self.type_ref_to_c(&fn_def.result);
            let params: Vec<String> = fn_def.params.iter()
                .map(|p| format!("{} {}", self.type_ref_to_c(&p.type_ref), p.name))
                .collect();
            let ps = if params.is_empty() { "void".to_string() } else { params.join(", ") };
            writeln!(out, "{} {}({}) {{", ret, fn_name, ps)?;

            // Register body computes
            for compute in &fn_def.body {
                self.compute_map.insert(compute.id.0.clone(), compute);
            }

            // Emit each body compute
            for compute in &fn_def.body {
                self.emit_compute_stmt(out, &compute.id.0, Some(fn_def), "    ")?;
            }

            // Return
            let ret_expr = self.resolve_value_expr(fn_def.returns.as_str(), Some(fn_def));
            writeln!(out, "    return {};", ret_expr)?;
            writeln!(out, "}}")?;
            writeln!(out)?;

            // Cleanup
            for compute in &fn_def.body {
                self.compute_map.remove(compute.id.as_str());
                self.emitted_values.remove(compute.id.as_str());
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Compute statement emission (used in F-Node bodies and main)
    // ------------------------------------------------------------------

    fn emit_compute_stmt(
        &mut self,
        out: &mut String,
        node_id: &str,
        fn_ctx: Option<&FnDef>,
        indent: &str,
    ) -> Result<(), CodegenCError> {
        if self.emitted_values.contains_key(node_id) {
            return Ok(());
        }
        self.emitted_values.insert(node_id.to_string(), true);

        let compute = self.compute_map.get(node_id)
            .ok_or_else(|| CodegenCError::UnresolvedNode(node_id.to_string()))?;
        let op = compute.op.clone();
        let var = Self::sanitize_id(node_id);

        match &op {
            ComputeOp::Const { value, type_ref, .. } => {
                let ct = self.type_ref_to_c(type_ref);
                let cv = Self::literal_to_c(value);
                writeln!(out, "{}{} {} = {};", indent, ct, var, cv)?;
            }
            ComputeOp::ConstBytes { .. } => {}
            ComputeOp::Arith { opcode, inputs, type_ref } => {
                for inp in inputs {
                    self.ensure_emitted_in_ctx(out, &inp.0, fn_ctx, indent)?;
                }
                let ct = self.type_ref_to_c(type_ref);
                let lhs = self.resolve_value_expr(&inputs[0].0, fn_ctx);
                let rhs = self.resolve_value_expr(&inputs[1].0, fn_ctx);
                let expr = Self::arith_to_c(opcode, &lhs, &rhs);
                writeln!(out, "{}{} {} = {};", indent, ct, var, expr)?;
            }
            ComputeOp::CallPure { target, inputs, type_ref } => {
                for inp in inputs {
                    self.ensure_emitted_in_ctx(out, &inp.0, fn_ctx, indent)?;
                }
                let ct = self.type_ref_to_c(type_ref);
                let safe = Self::sanitize_fn_name(target);
                let args: Vec<String> = inputs.iter()
                    .map(|i| self.resolve_value_expr(&i.0, fn_ctx))
                    .collect();
                writeln!(out, "{}{} {} = {}({});", indent, ct, var, safe, args.join(", "))?;
            }
            ComputeOp::Generic { name, inputs, type_ref, .. } => {
                for inp in inputs {
                    self.ensure_emitted_in_ctx(out, &inp.0, fn_ctx, indent)?;
                }
                let ct = self.type_ref_to_c(type_ref);
                let expr = self.generic_compute_to_c(name, inputs, fn_ctx);
                writeln!(out, "{}{} {} = {};", indent, ct, var, expr)?;
            }
            ComputeOp::StructGet { input, field, type_ref } => {
                self.ensure_emitted_in_ctx(out, &input.0, fn_ctx, indent)?;
                let ct = self.type_ref_to_c(type_ref);
                let inp = self.resolve_value_expr(&input.0, fn_ctx);
                writeln!(out, "{}{} {} = {}.{};", indent, ct, var, inp, field)?;
            }
            ComputeOp::StructSet { input, field, value, type_ref } => {
                self.ensure_emitted_in_ctx(out, &input.0, fn_ctx, indent)?;
                self.ensure_emitted_in_ctx(out, &value.0, fn_ctx, indent)?;
                let ct = self.type_ref_to_c(type_ref);
                let inp = self.resolve_value_expr(&input.0, fn_ctx);
                let val = self.resolve_value_expr(&value.0, fn_ctx);
                writeln!(out, "{}{} {} = {};", indent, ct, var, inp)?;
                writeln!(out, "{}{}.{} = {};", indent, var, field, val)?;
            }
            ComputeOp::VariantCreate { variant_type, tag, payload, type_ref } => {
                self.ensure_emitted_in_ctx(out, &payload.0, fn_ctx, indent)?;
                let ct = self.type_ref_to_c(type_ref);
                let tag_enum = self.variant_tag_enum_name(variant_type, tag);
                let pay = self.resolve_value_expr(&payload.0, fn_ctx);
                writeln!(out, "{}{} {};", indent, ct, var)?;
                writeln!(out, "{}{}.tag = {};", indent, var, tag_enum)?;
                if !self.is_void_type_for_payload(variant_type, tag) {
                    writeln!(out, "{}{}.payload.{} = {};", indent, var, tag, pay)?;
                }
            }
            ComputeOp::VariantIs { input, tag, .. } => {
                self.ensure_emitted_in_ctx(out, &input.0, fn_ctx, indent)?;
                let inp = self.resolve_value_expr(&input.0, fn_ctx);
                let tag_enum = self.variant_tag_enum_from_input(input, tag);
                writeln!(out, "{}bool {} = ({}.tag == {});", indent, var, inp, tag_enum)?;
            }
            ComputeOp::VariantGet { input, tag, type_ref } => {
                self.ensure_emitted_in_ctx(out, &input.0, fn_ctx, indent)?;
                let ct = self.type_ref_to_c(type_ref);
                let inp = self.resolve_value_expr(&input.0, fn_ctx);
                writeln!(out, "{}{} {} = {}.payload.{};", indent, ct, var, inp, tag)?;
            }
            ComputeOp::AtomicLoad { source, type_ref, .. } => {
                let ct = self.type_ref_to_c(type_ref);
                let src = Self::sanitize_id(&source.0);
                writeln!(out, "{}{} {} = {}; /* atomic load */", indent, ct, var, src)?;
            }
            ComputeOp::AtomicStore { target, value, .. } => {
                self.ensure_emitted_in_ctx(out, &value.0, fn_ctx, indent)?;
                let tgt = Self::sanitize_id(&target.0);
                let val = self.resolve_value_expr(&value.0, fn_ctx);
                writeln!(out, "{}{} = {}; /* atomic store */", indent, tgt, val)?;
            }
            ComputeOp::AtomicCas { .. } => {
                writeln!(out, "{}/* atomic CAS {} */", indent, node_id)?;
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // main() emission
    // ------------------------------------------------------------------

    fn emit_main(&mut self, out: &mut String) -> Result<(), CodegenCError> {
        writeln!(out, "int main(void) {{")?;
        self.emit_memory_allocs(out)?;
        let entry = self.program.entry.0.clone();
        self.emit_control_node(out, &entry, "    ")?;
        writeln!(out, "    return 0;")?;
        writeln!(out, "}}")?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // M-Node alloc emission
    // ------------------------------------------------------------------

    fn emit_memory_allocs(&self, out: &mut String) -> Result<(), CodegenCError> {
        for mem in &self.program.memories {
            if let MemoryOp::Alloc { type_ref, .. } = &mem.op {
                let var = Self::sanitize_id(&mem.id.0);
                if let TypeRef::Id { node } = type_ref {
                    if let Some(td) = self.type_map.get(&node.0) {
                        if let TypeBody::Array { element, max_length, .. } = &td.body {
                            let elem = self.type_ref_to_c(element);
                            writeln!(out, "    {} {}[{}];", elem, var, max_length)?;
                            writeln!(out, "    memset({}, 0, sizeof({}));", var, var)?;
                            continue;
                        }
                    }
                }
                let ct = self.type_ref_to_c(type_ref);
                if ct == "void" {
                    writeln!(out, "    uint8_t {}[8];", var)?;
                } else {
                    writeln!(out, "    {} {} = {{}};", ct, var)?;
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // K-Node emission
    // ------------------------------------------------------------------

    fn emit_control_node(
        &mut self,
        out: &mut String,
        node_id: &str,
        indent: &str,
    ) -> Result<(), CodegenCError> {
        if self.emitted_controls.contains_key(node_id) {
            return Ok(());
        }
        self.emitted_controls.insert(node_id.to_string(), true);

        let control = self.control_map.get(node_id)
            .ok_or_else(|| CodegenCError::UnresolvedNode(node_id.to_string()))?;
        let op = control.op.clone();

        match &op {
            ControlOp::Seq { steps } => {
                for step in steps {
                    match step.prefix() {
                        "E" => self.emit_effect_node(out, &step.0, indent)?,
                        "K" => self.emit_control_node(out, &step.0, indent)?,
                        "C" => self.emit_compute_stmt(out, &step.0, None, indent)?,
                        "M" => self.emit_memory_op(out, &step.0, indent)?,
                        _ => return Err(CodegenCError::Unsupported(format!("unsupported step: {}", step.0))),
                    }
                }
            }
            ControlOp::Branch { condition, true_branch, false_branch } => {
                self.ensure_emitted_in_ctx(out, &condition.0, None, indent)?;
                let cond = self.resolve_value_expr(&condition.0, None);
                writeln!(out, "{}if ({}) {{", indent, cond)?;
                let inner = format!("{}    ", indent);
                self.emit_control_node(out, &true_branch.0, &inner)?;
                writeln!(out, "{}}} else {{", indent)?;
                self.emit_control_node(out, &false_branch.0, &inner)?;
                writeln!(out, "{}}}", indent)?;
            }
            ControlOp::Loop { condition, body, state, .. } => {
                self.ensure_emitted_in_ctx(out, &state.0, None, indent)?;
                self.ensure_emitted_in_ctx(out, &condition.0, None, indent)?;
                let cond = self.resolve_value_expr(&condition.0, None);
                writeln!(out, "{}while ({}) {{", indent, cond)?;
                let inner = format!("{}    ", indent);
                self.emit_control_node(out, &body.0, &inner)?;
                writeln!(out, "{}}}", indent)?;
            }
            ControlOp::Par { branches, .. } => {
                for b in branches {
                    self.emit_control_node(out, &b.0, indent)?;
                }
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // E-Node emission
    // ------------------------------------------------------------------

    fn emit_effect_node(
        &mut self,
        out: &mut String,
        node_id: &str,
        indent: &str,
    ) -> Result<(), CodegenCError> {
        let effect = self.effect_map.get(node_id)
            .ok_or_else(|| CodegenCError::UnresolvedNode(node_id.to_string()))?;
        let op = effect.op.clone();

        match &op {
            EffectOp::Syscall { name, inputs, type_ref, success, .. } => {
                match name.as_str() {
                    "write" | "syscall_write" => self.emit_syscall_write(out, node_id, inputs, type_ref, indent)?,
                    "exit" | "syscall_exit" => self.emit_syscall_exit(out, inputs, indent)?,
                    "read" | "syscall_read" => self.emit_syscall_read(out, node_id, inputs, type_ref, indent)?,
                    "open" | "syscall_open" => self.emit_syscall_open(out, node_id, inputs, type_ref, indent)?,
                    "close" | "syscall_close" => self.emit_syscall_close(out, node_id, inputs, type_ref, indent)?,
                    "ioctl" | "syscall_ioctl" => self.emit_syscall_ioctl(out, node_id, inputs, type_ref, indent)?,
                    "nanosleep" | "syscall_nanosleep" => {
                        writeln!(out, "{}/* nanosleep */", indent)?;
                    }
                    _ => return Err(CodegenCError::Unsupported(format!("unsupported syscall: {name}"))),
                }
                if let Some(succ) = success {
                    self.emit_control_node(out, &succ.0, indent)?;
                }
            }
            EffectOp::CallExtern { target, inputs, type_ref, success, .. } => {
                self.emit_call_extern(out, node_id, &target.0, inputs, type_ref, indent)?;
                self.emit_control_node(out, &success.0, indent)?;
            }
            EffectOp::Generic { name, .. } => {
                return Err(CodegenCError::Unsupported(format!("generic effect: {name}")));
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall implementations
    // ------------------------------------------------------------------

    fn emit_syscall_write(&mut self, out: &mut String, node_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        if inputs.len() != 3 { return Err(CodegenCError::Unsupported("write expects 3 inputs".into())); }
        self.ensure_emitted_in_ctx(out, &inputs[0].0, None, indent)?;
        self.ensure_emitted_in_ctx(out, &inputs[2].0, None, indent)?;
        let fd = self.resolve_value_expr(&inputs[0].0, None);
        let buf = self.resolve_pointer_expr(&inputs[1].0);
        let len = self.resolve_value_expr(&inputs[2].0, None);
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = write((int){}, {}, (size_t){});", indent, ct, var, fd, buf, len)?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}write((int){}, {}, (size_t){});", indent, fd, buf, len)?;
        }
        Ok(())
    }

    fn emit_syscall_exit(&mut self, out: &mut String, inputs: &[NodeRef], indent: &str) -> Result<(), CodegenCError> {
        if inputs.is_empty() { return Err(CodegenCError::Unsupported("exit expects 1 input".into())); }
        self.ensure_emitted_in_ctx(out, &inputs[0].0, None, indent)?;
        let code = self.resolve_value_expr(&inputs[0].0, None);
        writeln!(out, "{}_exit((int){});", indent, code)?;
        Ok(())
    }

    fn emit_syscall_read(&mut self, out: &mut String, node_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        if inputs.len() != 3 { return Err(CodegenCError::Unsupported("read expects 3 inputs".into())); }
        self.ensure_emitted_in_ctx(out, &inputs[0].0, None, indent)?;
        self.ensure_emitted_in_ctx(out, &inputs[2].0, None, indent)?;
        let fd = self.resolve_value_expr(&inputs[0].0, None);
        let buf = self.resolve_pointer_expr(&inputs[1].0);
        let len = self.resolve_value_expr(&inputs[2].0, None);
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = read((int){}, {}, (size_t){});", indent, ct, var, fd, buf, len)?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}read((int){}, {}, (size_t){});", indent, fd, buf, len)?;
        }
        Ok(())
    }

    fn emit_syscall_open(&mut self, out: &mut String, node_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        if inputs.len() < 2 { return Err(CodegenCError::Unsupported("open expects 2+ inputs".into())); }
        self.ensure_emitted_in_ctx(out, &inputs[1].0, None, indent)?;
        let path = self.resolve_pointer_expr(&inputs[0].0);
        let flags = self.resolve_value_expr(&inputs[1].0, None);
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = open((const char*){}, (int){});", indent, ct, var, path, flags)?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}open((const char*){}, (int){});", indent, path, flags)?;
        }
        Ok(())
    }

    fn emit_syscall_close(&mut self, out: &mut String, node_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        if inputs.is_empty() { return Err(CodegenCError::Unsupported("close expects 1 input".into())); }
        self.ensure_emitted_in_ctx(out, &inputs[0].0, None, indent)?;
        let fd = self.resolve_value_expr(&inputs[0].0, None);
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = close((int){});", indent, ct, var, fd)?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}close((int){});", indent, fd)?;
        }
        Ok(())
    }

    fn emit_syscall_ioctl(&mut self, out: &mut String, node_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        if inputs.len() < 2 { return Err(CodegenCError::Unsupported("ioctl expects 2+ inputs".into())); }
        for inp in inputs { self.ensure_emitted_in_ctx(out, &inp.0, None, indent)?; }
        let fd = self.resolve_value_expr(&inputs[0].0, None);
        let req = self.resolve_value_expr(&inputs[1].0, None);
        let extra: Vec<String> = inputs[2..].iter().map(|i| self.resolve_value_expr(&i.0, None)).collect();
        let mut args = format!("(int){}, (unsigned long){}", fd, req);
        for e in &extra { write!(args, ", {}", e)?; }
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = ioctl({});", indent, ct, var, args)?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}ioctl({});", indent, args)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // CallExtern emission
    // ------------------------------------------------------------------

    fn emit_call_extern(&mut self, out: &mut String, node_id: &str, target_id: &str, inputs: &[NodeRef], type_ref: &TypeRef, indent: &str) -> Result<(), CodegenCError> {
        let func_name = self.extern_map.get(target_id).map(|e| e.name.clone()).unwrap_or_else(|| target_id.to_string());
        for inp in inputs { self.ensure_emitted_in_ctx(out, &inp.0, None, indent)?; }
        let args: Vec<String> = inputs.iter().map(|i| {
            if self.is_pointer_node(&i.0) { self.resolve_pointer_expr(&i.0) }
            else { self.resolve_value_expr(&i.0, None) }
        }).collect();
        let var = Self::sanitize_id(node_id);
        if !self.is_void_type(type_ref) {
            let ct = self.type_ref_to_c(type_ref);
            writeln!(out, "{}{} {} = {}({});", indent, ct, var, func_name, args.join(", "))?;
            self.emitted_values.insert(node_id.to_string(), true);
        } else {
            writeln!(out, "{}{}({});", indent, func_name, args.join(", "))?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // M-Node load/store emission
    // ------------------------------------------------------------------

    fn emit_memory_op(&mut self, out: &mut String, node_id: &str, indent: &str) -> Result<(), CodegenCError> {
        let mem = self.memory_map.get(node_id)
            .ok_or_else(|| CodegenCError::UnresolvedNode(node_id.to_string()))?;
        let op = mem.op.clone();

        match &op {
            MemoryOp::Alloc { .. } => {}
            MemoryOp::Store { target, index, value } => {
                self.ensure_emitted_in_ctx(out, &index.0, None, indent)?;
                self.ensure_emitted_in_ctx(out, &value.0, None, indent)?;
                let tgt = Self::sanitize_id(&target.0);
                let idx = self.resolve_value_expr(&index.0, None);
                let val = self.resolve_value_expr(&value.0, None);
                if self.is_array_alloc(&target.0) {
                    writeln!(out, "{}{}[(int){}] = {};", indent, tgt, idx, val)?;
                } else {
                    let es = self.get_memory_element_size(&target.0);
                    writeln!(out, "{}memcpy((uint8_t*)&{} + (int){} * {}, &({}), sizeof({}));", indent, tgt, idx, es, val, val)?;
                }
            }
            MemoryOp::Load { source, index, type_ref } => {
                self.ensure_emitted_in_ctx(out, &index.0, None, indent)?;
                let var = Self::sanitize_id(node_id);
                let src = Self::sanitize_id(&source.0);
                let idx = self.resolve_value_expr(&index.0, None);
                let ct = self.type_ref_to_c(type_ref);
                if self.is_array_alloc(&source.0) {
                    writeln!(out, "{}{} {} = {}[(int){}];", indent, ct, var, src, idx)?;
                } else {
                    let es = self.get_memory_element_size(&source.0);
                    writeln!(out, "{}{} {};", indent, ct, var)?;
                    writeln!(out, "{}memcpy(&{}, (uint8_t*)&{} + (int){} * {}, sizeof({}));", indent, var, src, idx, es, var)?;
                }
                self.emitted_values.insert(node_id.to_string(), true);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn ensure_emitted_in_ctx(&mut self, out: &mut String, node_id: &str, fn_ctx: Option<&FnDef>, indent: &str) -> Result<(), CodegenCError> {
        if self.emitted_values.contains_key(node_id) { return Ok(()); }
        // P: prefix params are always available
        if node_id.starts_with("P:") { return Ok(()); }
        if self.compute_map.contains_key(node_id) {
            self.emit_compute_stmt(out, node_id, fn_ctx, indent)?;
        }
        if self.memory_map.contains_key(node_id) {
            self.emit_memory_op(out, node_id, indent)?;
        }
        Ok(())
    }

    fn resolve_value_expr(&self, node_id: &str, fn_ctx: Option<&FnDef>) -> String {
        if node_id.starts_with("P:") {
            return node_id[2..].to_string();
        }
        if let Some(fn_def) = fn_ctx {
            for p in &fn_def.params {
                if node_id == format!("P:{}", p.name) {
                    return p.name.clone();
                }
            }
        }
        if let Some(compute) = self.compute_map.get(node_id) {
            if let ComputeOp::Const { value, .. } = &compute.op {
                if !self.emitted_values.contains_key(node_id) {
                    return Self::literal_to_c(value);
                }
            }
        }
        Self::sanitize_id(node_id)
    }

    fn resolve_pointer_expr(&self, node_id: &str) -> String {
        if let Some(c) = self.compute_map.get(node_id) {
            if matches!(c.op, ComputeOp::ConstBytes { .. }) {
                return format!("const_{}", Self::sanitize_id(node_id));
            }
        }
        if self.memory_map.contains_key(node_id) {
            let var = Self::sanitize_id(node_id);
            if self.is_array_alloc(node_id) { return var; }
            return format!("&{}", var);
        }
        format!("(void*)(uintptr_t){}", Self::sanitize_id(node_id))
    }

    fn is_pointer_node(&self, node_id: &str) -> bool {
        if let Some(c) = self.compute_map.get(node_id) {
            if matches!(c.op, ComputeOp::ConstBytes { .. }) { return true; }
        }
        self.memory_map.contains_key(node_id)
    }

    fn is_array_alloc(&self, node_id: &str) -> bool {
        for mem in &self.program.memories {
            if mem.id.0 == node_id {
                if let MemoryOp::Alloc { type_ref, .. } = &mem.op {
                    if let TypeRef::Id { node } = type_ref {
                        if let Some(td) = self.type_map.get(&node.0) {
                            return matches!(td.body, TypeBody::Array { .. });
                        }
                    }
                }
                return false;
            }
        }
        false
    }

    // ------------------------------------------------------------------
    // Literal conversion
    // ------------------------------------------------------------------

    fn literal_to_c(lit: &Literal) -> String {
        match lit {
            Literal::Integer { value } => {
                if *value > i32::MAX as i64 || *value < i32::MIN as i64 {
                    format!("{}LL", value)
                } else {
                    format!("{}", value)
                }
            }
            Literal::Float { value } => format!("{}", value),
            Literal::Bool { value } => if *value { "true".into() } else { "false".into() },
            Literal::Str { value } => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        }
    }

    // ------------------------------------------------------------------
    // Arithmetic expression conversion
    // ------------------------------------------------------------------

    fn arith_to_c(opcode: &str, lhs: &str, rhs: &str) -> String {
        match opcode {
            "add" => format!("({} + {})", lhs, rhs),
            "sub" => format!("({} - {})", lhs, rhs),
            "mul" => format!("({} * {})", lhs, rhs),
            "div" | "sdiv" => format!("({} / {})", lhs, rhs),
            "udiv" => format!("((unsigned){} / (unsigned){})", lhs, rhs),
            "mod" | "srem" => format!("({} % {})", lhs, rhs),
            "urem" => format!("((unsigned){} % (unsigned){})", lhs, rhs),
            "and" => format!("({} & {})", lhs, rhs),
            "or" => format!("({} | {})", lhs, rhs),
            "xor" => format!("({} ^ {})", lhs, rhs),
            "shl" => format!("({} << {})", lhs, rhs),
            "shr" | "ashr" => format!("({} >> {})", lhs, rhs),
            "lshr" => format!("((unsigned){} >> {})", lhs, rhs),
            "gt" | "sgt" => format!("({} > {})", lhs, rhs),
            "lt" | "slt" => format!("({} < {})", lhs, rhs),
            "gte" | "sge" => format!("({} >= {})", lhs, rhs),
            "lte" | "sle" => format!("({} <= {})", lhs, rhs),
            "eq" => format!("({} == {})", lhs, rhs),
            "neq" => format!("({} != {})", lhs, rhs),
            "ugt" => format!("((unsigned){} > (unsigned){})", lhs, rhs),
            "ult" => format!("((unsigned){} < (unsigned){})", lhs, rhs),
            "uge" => format!("((unsigned){} >= (unsigned){})", lhs, rhs),
            "ule" => format!("((unsigned){} <= (unsigned){})", lhs, rhs),
            _ => format!("/* unsupported op: {} */ 0", opcode),
        }
    }

    // ------------------------------------------------------------------
    // Generic compute expression conversion
    // ------------------------------------------------------------------

    fn generic_compute_to_c(&self, name: &str, inputs: &[NodeRef], fn_ctx: Option<&FnDef>) -> String {
        match name {
            "gt" | "sgt" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} > {})", a, b) }
            "lt" | "slt" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} < {})", a, b) }
            "gte" | "sge" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} >= {})", a, b) }
            "lte" | "sle" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} <= {})", a, b) }
            "eq" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} == {})", a, b) }
            "neq" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("({} != {})", a, b) }
            "not" => { let a = self.resolve_value_expr(&inputs[0].0, fn_ctx); format!("(~{})", a) }
            "neg" => { let a = self.resolve_value_expr(&inputs[0].0, fn_ctx); format!("(-{})", a) }
            "abs" => { let a = self.resolve_value_expr(&inputs[0].0, fn_ctx); format!("(({a}) < 0 ? -({a}) : ({a}))") }
            "min" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("(({a}) < ({b}) ? ({a}) : ({b}))") }
            "max" => { let (a, b) = self.two_inputs(inputs, fn_ctx); format!("(({a}) > ({b}) ? ({a}) : ({b}))") }
            "clamp" if inputs.len() >= 3 => {
                let v = self.resolve_value_expr(&inputs[0].0, fn_ctx);
                let lo = self.resolve_value_expr(&inputs[1].0, fn_ctx);
                let hi = self.resolve_value_expr(&inputs[2].0, fn_ctx);
                format!("(({v}) < ({lo}) ? ({lo}) : (({v}) > ({hi}) ? ({hi}) : ({v})))")
            }
            _ => format!("0 /* unsupported generic: {} */", name),
        }
    }

    fn two_inputs(&self, inputs: &[NodeRef], fn_ctx: Option<&FnDef>) -> (String, String) {
        (self.resolve_value_expr(&inputs[0].0, fn_ctx), self.resolve_value_expr(&inputs[1].0, fn_ctx))
    }

    // ------------------------------------------------------------------
    // Variant helpers
    // ------------------------------------------------------------------

    fn variant_tag_enum_name(&self, variant_type: &TypeRef, tag: &str) -> String {
        match variant_type {
            TypeRef::Id { node } => format!("{}_{}", Self::sanitize_id(&node.0).to_uppercase(), tag.to_uppercase()),
            TypeRef::Builtin { name } => format!("{}_{}", name.to_uppercase(), tag.to_uppercase()),
        }
    }

    fn variant_tag_enum_from_input(&self, input: &NodeRef, tag: &str) -> String {
        if let Some(c) = self.compute_map.get(&input.0) {
            let tr = match &c.op {
                ComputeOp::VariantCreate { type_ref, .. } => Some(type_ref),
                ComputeOp::Const { type_ref, .. } => Some(type_ref),
                ComputeOp::Generic { type_ref, .. } => Some(type_ref),
                _ => None,
            };
            if let Some(t) = tr { return self.variant_tag_enum_name(t, tag); }
        }
        format!("TAG_{}", tag.to_uppercase())
    }

    fn is_void_type_for_payload(&self, variant_type: &TypeRef, tag: &str) -> bool {
        if let TypeRef::Id { node } = variant_type {
            if let Some(td) = self.type_map.get(&node.0) {
                if let TypeBody::Variant { cases } = &td.body {
                    for case in cases {
                        if case.tag == tag { return self.is_void_type(&case.payload); }
                    }
                }
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline;

    fn parse_and_gen(source: &str) -> Result<String, String> {
        let result = pipeline::run_check(source);
        let ast = result.ast.ok_or("parse failed")?;
        codegen_c(&ast).map_err(|e| e.to_string())
    }

    #[test]
    fn test_minimal() {
        let source = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
        let result = parse_and_gen(source).expect("codegen_c failed");
        assert!(result.contains("#include <stdint.h>"));
        assert!(result.contains("int main(void)"));
        assert!(result.contains("_exit"));
    }

    #[test]
    fn test_hello_world() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/hello_world.ftl"),
        ).expect("read hello_world.ftl");
        let result = parse_and_gen(&source).expect("codegen_c failed");
        assert!(result.contains("write("));
        assert!(result.contains("_exit("));
        assert!(result.contains("const_C_c1"));
    }

    #[test]
    fn test_fn_basic() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/fn_basic.ftl"),
        ).expect("read fn_basic.ftl");
        let result = parse_and_gen(&source).expect("codegen_c failed");
        assert!(result.contains("int32_t ftl_double("));
        assert!(result.contains("return"));
    }

    #[test]
    fn test_arith_ops() {
        let source = r#"
T:i32 = integer { bits: 32, signed: true }
C:c1 = const { value: 10, type: T:i32 }
C:c2 = const { value: 3, type: T:i32 }
C:c3 = add { inputs: [C:c1, C:c2], type: T:i32 }
C:c4 = sub { inputs: [C:c1, C:c2], type: T:i32 }
C:c5 = mul { inputs: [C:c1, C:c2], type: T:i32 }
K:main = seq { steps: [C:c3, C:c4, C:c5] }
entry: K:main
"#;
        let result = parse_and_gen(source).expect("codegen_c failed");
        assert!(result.contains("+"));
        assert!(result.contains("-"));
        assert!(result.contains("*"));
    }

    #[test]
    fn test_struct_type() {
        let source = r#"
T:i32 = integer { bits: 32, signed: true }
T:pos = struct { fields: [x: T:i32, y: T:i32] }
C:c1 = const { value: 0, type: T:i32 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:i32, effects: [PROC] }
K:main = seq { steps: [E:d1] }
entry: K:main
"#;
        let result = parse_and_gen(source).expect("codegen_c failed");
        assert!(result.contains("typedef struct {"));
        assert!(result.contains("int32_t x;"));
        assert!(result.contains("int32_t y;"));
        assert!(result.contains("T_pos_t;"));
    }

    #[test]
    fn test_variant_type() {
        let source = r#"
T:i32 = integer { bits: 32, signed: true }
T:result = variant { cases: [ok: T:i32, err: T:i32] }
C:c1 = const { value: 0, type: T:i32 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:i32, effects: [PROC] }
K:main = seq { steps: [E:d1] }
entry: K:main
"#;
        let result = parse_and_gen(source).expect("codegen_c failed");
        assert!(result.contains("enum T_result_tag"));
        assert!(result.contains("T_RESULT_OK"));
        assert!(result.contains("T_RESULT_ERR"));
        assert!(result.contains("int32_t tag;"));
        assert!(result.contains("union {"));
    }
}
