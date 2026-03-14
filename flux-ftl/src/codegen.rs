// ---------------------------------------------------------------------------
// codegen.rs — LLVM IR generation from FTL AST via inkwell
// ---------------------------------------------------------------------------
//
// Translates a parsed FTL Program into LLVM IR using inkwell (LLVM 14).
// Supports:
//   - Const / ConstBytes C-Nodes
//   - Arith C-Nodes (add, sub, mul, div, mod)
//   - Comparison C-Nodes (gt, lt, gte, lte, eq, neq)
//   - AtomicLoad / AtomicStore / AtomicCas C-Nodes
//   - Syscall E-Nodes (write, exit, read, open, close, ioctl, nanosleep)
//   - CallExtern E-Nodes (FFI calls)
//   - Seq / Branch / Loop / Par K-Nodes
//   - Alloc / Load / Store M-Nodes (memory operations)
//   - Extern X-Node declarations
//
// The generated IR links against libc for write(), _exit(), and other syscalls.
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::AddressSpace;
use inkwell::AtomicOrdering;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;

use crate::ast::*;
use crate::optimizer;

// ---------------------------------------------------------------------------
// Public configuration types
// ---------------------------------------------------------------------------

/// Controls code generation behavior.
#[derive(Debug, Clone)]
pub struct CodegenConfig {
    /// LLVM target triple (default: host triple).
    pub target_triple: String,
    /// Optimization level (0-3).
    pub opt_level: OptLevel,
    /// Desired output format.
    pub output_format: OutputFormat,
}

/// Optimization level mapping.
#[derive(Debug, Clone, Copy)]
pub enum OptLevel {
    None,
    Less,
    Default,
    Aggressive,
}

/// Output format selector.
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    ObjectFile,
    Assembly,
    LlvmIr,
}

/// Successful codegen output.
pub struct CodegenResult {
    /// Textual LLVM IR representation.
    pub llvm_ir: String,
    /// Compiled bytes (object file, assembly, or IR text as bytes).
    pub output_bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during code generation.
#[derive(Debug)]
pub enum CodegenError {
    /// A referenced node was not found in the program.
    UnresolvedNode(String),
    /// LLVM target initialization failed.
    TargetInitFailed(String),
    /// LLVM module verification failed.
    VerificationFailed(String),
    /// Object file emission failed.
    EmitFailed(String),
    /// Unsupported AST construct.
    Unsupported(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::UnresolvedNode(msg) => write!(f, "unresolved node: {msg}"),
            CodegenError::TargetInitFailed(msg) => write!(f, "target init failed: {msg}"),
            CodegenError::VerificationFailed(msg) => write!(f, "verification failed: {msg}"),
            CodegenError::EmitFailed(msg) => write!(f, "emit failed: {msg}"),
            CodegenError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
        }
    }
}

impl std::error::Error for CodegenError {}

// ---------------------------------------------------------------------------
// Default config
// ---------------------------------------------------------------------------

impl Default for CodegenConfig {
    fn default() -> Self {
        Self {
            target_triple: TargetMachine::get_default_triple()
                .as_str()
                .to_string_lossy()
                .into_owned(),
            opt_level: OptLevel::None,
            output_format: OutputFormat::ObjectFile,
        }
    }
}

impl OptLevel {
    fn to_inkwell(self) -> OptimizationLevel {
        match self {
            OptLevel::None => OptimizationLevel::None,
            OptLevel::Less => OptimizationLevel::Less,
            OptLevel::Default => OptimizationLevel::Default,
            OptLevel::Aggressive => OptimizationLevel::Aggressive,
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Generate LLVM IR (and optionally an object file) from a parsed FTL program.
pub fn codegen(program: &Program, config: &CodegenConfig) -> Result<CodegenResult, CodegenError> {
    let context = Context::create();
    let mut generator = CodeGenerator::new(&context, program, config)?;
    generator.emit_program()?;

    // Run LLVM optimization passes on the main function if opt_level > 0
    let llvm_opt_level = match config.opt_level {
        OptLevel::None => 0u8,
        OptLevel::Less => 1,
        OptLevel::Default => 2,
        OptLevel::Aggressive => 3,
    };
    if llvm_opt_level > 0
        && let Some(main_fn) = generator.module.get_function("main")
    {
        optimizer::optimize_llvm_function(&generator.module, main_fn, llvm_opt_level);
    }

    generator.finish()
}

// ---------------------------------------------------------------------------
// Internal code generator
// ---------------------------------------------------------------------------

struct CodeGenerator<'ctx, 'prog> {
    context: &'ctx Context,
    module: Module<'ctx>,
    program: &'prog Program,
    config: CodegenConfig,

    // Lookup tables built from the AST
    compute_map: HashMap<String, &'prog ComputeDef>,
    effect_map: HashMap<String, &'prog EffectDef>,
    control_map: HashMap<String, &'prog ControlDef>,
    memory_map: HashMap<String, &'prog MemoryDef>,
    #[allow(dead_code)]
    extern_map: HashMap<String, &'prog ExternDef>,

    // LLVM values produced during emission
    values: HashMap<String, BasicValueEnum<'ctx>>,
    // Pointers for const_bytes globals and M-Node allocations
    pointers: HashMap<String, PointerValue<'ctx>>,
    // Declared libc / extern functions
    functions: HashMap<String, FunctionValue<'ctx>>,
    // Track which K-nodes have already been emitted (for cycle prevention)
    emitted_controls: HashMap<String, bool>,
}

impl<'ctx, 'prog> CodeGenerator<'ctx, 'prog> {
    fn new(
        context: &'ctx Context,
        program: &'prog Program,
        config: &CodegenConfig,
    ) -> Result<Self, CodegenError> {
        let module = context.create_module("flux_module");

        // Build lookup maps
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
        let mut extern_map = HashMap::new();
        for x in &program.externs {
            extern_map.insert(x.id.0.clone(), x);
        }

        Ok(Self {
            context,
            module,
            program,
            config: config.clone(),
            compute_map,
            effect_map,
            control_map,
            memory_map,
            extern_map,
            values: HashMap::new(),
            pointers: HashMap::new(),
            functions: HashMap::new(),
            emitted_controls: HashMap::new(),
        })
    }

    // ------------------------------------------------------------------
    // Program emission
    // ------------------------------------------------------------------

    fn emit_program(&mut self) -> Result<(), CodegenError> {
        // 1. Declare libc helpers we may need
        self.declare_libc_functions();

        // 2. Declare extern (X-Node) functions
        self.declare_extern_functions();

        // 3. Emit global constants (ConstBytes)
        self.emit_global_constants()?;

        // 4. Build `main` function
        self.emit_main()?;

        // 5. Verify module
        self.module
            .verify()
            .map_err(|e| CodegenError::VerificationFailed(e.to_string()))?;

        Ok(())
    }

    // ------------------------------------------------------------------
    // Declare libc wrappers
    // ------------------------------------------------------------------

    fn declare_libc_functions(&mut self) {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // ssize_t write(int fd, const void *buf, size_t count)
        let write_ty = i64_type.fn_type(
            &[i32_type.into(), i8_ptr_type.into(), i64_type.into()],
            false,
        );
        let write_fn = self.module.add_function("write", write_ty, None);
        self.functions.insert("write".to_string(), write_fn);

        // void _exit(int status)
        let exit_ty = void_type.fn_type(&[i32_type.into()], false);
        let exit_fn = self.module.add_function("_exit", exit_ty, None);
        self.functions.insert("_exit".to_string(), exit_fn);

        // ssize_t read(int fd, void *buf, size_t count)
        let read_ty = i64_type.fn_type(
            &[i32_type.into(), i8_ptr_type.into(), i64_type.into()],
            false,
        );
        let read_fn = self.module.add_function("read", read_ty, None);
        self.functions.insert("read".to_string(), read_fn);

        // int open(const char *pathname, int flags)
        let open_ty = i32_type.fn_type(&[i8_ptr_type.into(), i32_type.into()], false);
        let open_fn = self.module.add_function("open", open_ty, None);
        self.functions.insert("open".to_string(), open_fn);

        // int close(int fd)
        let close_ty = i32_type.fn_type(&[i32_type.into()], false);
        let close_fn = self.module.add_function("close", close_ty, None);
        self.functions.insert("close".to_string(), close_fn);

        // int ioctl(int fd, unsigned long request, ...)
        let ioctl_ty = i32_type.fn_type(&[i32_type.into(), i64_type.into()], true);
        let ioctl_fn = self.module.add_function("ioctl", ioctl_ty, None);
        self.functions.insert("ioctl".to_string(), ioctl_fn);

        // int nanosleep(const struct timespec *req, struct timespec *rem)
        let nanosleep_ty =
            i32_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let nanosleep_fn = self.module.add_function("nanosleep", nanosleep_ty, None);
        self.functions.insert("nanosleep".to_string(), nanosleep_fn);
    }

    // ------------------------------------------------------------------
    // Declare X-Node extern functions
    // ------------------------------------------------------------------

    fn declare_extern_functions(&mut self) {
        for ext in &self.program.externs {
            // Skip if we already declared it (e.g. write, _exit)
            if self.functions.contains_key(&ext.name) {
                continue;
            }

            let ret_type = self.type_ref_to_llvm(&ext.result);
            let param_types: Vec<_> = ext
                .params
                .iter()
                .map(|p| self.type_ref_to_basic_metadata(p))
                .collect();

            let fn_type = match ret_type {
                Some(basic) => match basic {
                    inkwell::types::BasicTypeEnum::IntType(t) => t.fn_type(&param_types, false),
                    inkwell::types::BasicTypeEnum::FloatType(t) => t.fn_type(&param_types, false),
                    inkwell::types::BasicTypeEnum::PointerType(t) => {
                        t.fn_type(&param_types, false)
                    }
                    inkwell::types::BasicTypeEnum::ArrayType(t) => t.fn_type(&param_types, false),
                    inkwell::types::BasicTypeEnum::StructType(t) => {
                        t.fn_type(&param_types, false)
                    }
                    inkwell::types::BasicTypeEnum::VectorType(t) => {
                        t.fn_type(&param_types, false)
                    }
                },
                None => self.context.void_type().fn_type(&param_types, false),
            };

            let func = self.module.add_function(&ext.name, fn_type, None);
            self.functions.insert(ext.name.clone(), func);
            // Also store by X-node id so we can look up by NodeRef
            self.functions.insert(ext.id.0.clone(), func);
        }
    }

    // ------------------------------------------------------------------
    // Type conversions
    // ------------------------------------------------------------------

    /// Convert a TypeRef to an LLVM basic type. Returns None for unit/void.
    fn type_ref_to_llvm(
        &self,
        type_ref: &TypeRef,
    ) -> Option<inkwell::types::BasicTypeEnum<'ctx>> {
        match type_ref {
            TypeRef::Builtin { name } => self.builtin_type_to_llvm(name),
            TypeRef::Id { node } => {
                // Look up the type definition
                let type_def = self.program.types.iter().find(|t| t.id == *node);
                match type_def {
                    Some(td) => self.type_body_to_llvm(&td.body),
                    None => Some(self.context.i64_type().into()),
                }
            }
        }
    }

    fn builtin_type_to_llvm(
        &self,
        name: &str,
    ) -> Option<inkwell::types::BasicTypeEnum<'ctx>> {
        match name {
            "unit" | "void" => None,
            "bool" | "boolean" => Some(self.context.bool_type().into()),
            "u8" | "i8" => Some(self.context.i8_type().into()),
            "u16" | "i16" => Some(self.context.i16_type().into()),
            "u32" | "i32" => Some(self.context.i32_type().into()),
            "u64" | "i64" => Some(self.context.i64_type().into()),
            "f32" => Some(self.context.f32_type().into()),
            "f64" => Some(self.context.f64_type().into()),
            _ => Some(self.context.i64_type().into()),
        }
    }

    fn type_body_to_llvm(
        &self,
        body: &TypeBody,
    ) -> Option<inkwell::types::BasicTypeEnum<'ctx>> {
        match body {
            TypeBody::Integer { bits, .. } => match bits {
                1 => Some(self.context.bool_type().into()),
                8 => Some(self.context.i8_type().into()),
                16 => Some(self.context.i16_type().into()),
                32 => Some(self.context.i32_type().into()),
                64 => Some(self.context.i64_type().into()),
                n => Some(self.context.custom_width_int_type(*n).into()),
            },
            TypeBody::Float { bits } => match bits {
                32 => Some(self.context.f32_type().into()),
                64 => Some(self.context.f64_type().into()),
                _ => Some(self.context.f64_type().into()),
            },
            TypeBody::Boolean => Some(self.context.bool_type().into()),
            TypeBody::Unit => None,
            TypeBody::Opaque { .. } => {
                // Opaque types are represented as i8* (pointer)
                Some(
                    self.context
                        .i8_type()
                        .ptr_type(AddressSpace::default())
                        .into(),
                )
            }
            TypeBody::Array { .. } => {
                // Arrays are passed as pointers
                Some(
                    self.context
                        .i8_type()
                        .ptr_type(AddressSpace::default())
                        .into(),
                )
            }
            TypeBody::Struct { .. } | TypeBody::Variant { .. } | TypeBody::Fn { .. } => {
                // For now, represent complex types as i64
                Some(self.context.i64_type().into())
            }
        }
    }

    fn type_ref_to_basic_metadata(
        &self,
        type_ref: &TypeRef,
    ) -> inkwell::types::BasicMetadataTypeEnum<'ctx> {
        match self.type_ref_to_llvm(type_ref) {
            Some(t) => t.into(),
            None => self.context.i32_type().into(), // fallback for void params
        }
    }

    /// Get the byte size for a given TypeRef. Falls back to 8 bytes (i64).
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
                let type_def = self.program.types.iter().find(|t| t.id == *node);
                match type_def {
                    Some(td) => match &td.body {
                        TypeBody::Integer { bits, .. } | TypeBody::Float { bits } => {
                            (*bits as u64).div_ceil(8)
                        }
                        TypeBody::Boolean => 1,
                        TypeBody::Unit => 0,
                        TypeBody::Opaque { size, .. } => *size as u64,
                        TypeBody::Array { max_length, .. } => {
                            *max_length as u64 * 8
                        }
                        _ => 8,
                    },
                    None => 8,
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Emit global constants (ConstBytes)
    // ------------------------------------------------------------------

    fn emit_global_constants(&mut self) -> Result<(), CodegenError> {
        for compute in &self.program.computes {
            if let ComputeOp::ConstBytes { value, .. } = &compute.op {
                let byte_values: Vec<_> = value
                    .iter()
                    .map(|b| self.context.i8_type().const_int(*b as u64, false))
                    .collect();
                let array_val = self.context.i8_type().const_array(&byte_values);
                let global = self.module.add_global(
                    self.context.i8_type().array_type(value.len() as u32),
                    Some(AddressSpace::default()),
                    &format!("const_{}", compute.id.0.replace(':', "_")),
                );
                global.set_initializer(&array_val);
                global.set_constant(true);
                global.set_unnamed_addr(true);

                self.pointers.insert(
                    compute.id.0.clone(),
                    global.as_pointer_value(),
                );
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Emit the `main` function
    // ------------------------------------------------------------------

    fn emit_main(&mut self) -> Result<(), CodegenError> {
        let i32_type = self.context.i32_type();
        let main_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);

        let entry_bb = self.context.append_basic_block(main_fn, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry_bb);

        // Emit M-Node allocations at the top of the function
        self.emit_memory_allocs(main_fn, &builder)?;

        // Emit the entry K-node
        self.emit_control_node(&self.program.entry.0.clone(), main_fn, &builder)?;

        // If the builder's current block has no terminator, add `ret i32 0`
        let current_block = builder.get_insert_block();
        if let Some(block) = current_block
            && block.get_terminator().is_none()
        {
            builder.build_return(Some(&i32_type.const_int(0, false)))
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // M-Node allocation emission (alloca at function entry)
    // ------------------------------------------------------------------

    fn emit_memory_allocs(
        &mut self,
        _function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        let alloc_defs: Vec<_> = self
            .program
            .memories
            .iter()
            .filter(|m| matches!(m.op, MemoryOp::Alloc { .. }))
            .cloned()
            .collect();

        for mem_def in &alloc_defs {
            if let MemoryOp::Alloc { ref type_ref, .. } = mem_def.op {
                let size = self.type_ref_byte_size(type_ref);
                let llvm_type = self.type_ref_to_llvm(type_ref);
                let ptr = match llvm_type {
                    Some(inkwell::types::BasicTypeEnum::IntType(t)) => {
                        builder
                            .build_alloca(t, &mem_def.id.0.replace(':', "_"))
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    }
                    _ => {
                        let alloc_size = if size == 0 { 8 } else { size };
                        let arr_ty = self.context.i8_type().array_type(alloc_size as u32);
                        builder
                            .build_alloca(arr_ty, &mem_def.id.0.replace(':', "_"))
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    }
                };
                self.pointers.insert(mem_def.id.0.clone(), ptr);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // M-Node emission (load / store)
    // ------------------------------------------------------------------

    fn emit_memory_op(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        let mem_def = self
            .memory_map
            .get(node_id)
            .ok_or_else(|| CodegenError::UnresolvedNode(node_id.to_string()))?;
        let op = mem_def.op.clone();

        match &op {
            MemoryOp::Alloc { .. } => {
                // Allocs are emitted at function entry, nothing to do here
            }
            MemoryOp::Store {
                target,
                index,
                value,
            } => {
                let target_ptr = self
                    .pointers
                    .get(&target.0)
                    .copied()
                    .ok_or_else(|| {
                        CodegenError::UnresolvedNode(format!("M-node pointer: {}", target.0))
                    })?;
                let idx_val = self.resolve_value(&index.0, function, builder)?;
                let store_val = self.resolve_value(&value.0, function, builder)?;

                let i8_ptr = builder
                    .build_bitcast(
                        target_ptr,
                        self.context.i8_type().ptr_type(AddressSpace::default()),
                        "store_base",
                    )
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    .into_pointer_value();

                let element_size = self.context.i64_type().const_int(8, false);
                let idx_i64 = self.int_to_i64(idx_val.into_int_value(), builder)?;
                let byte_offset = builder
                    .build_int_mul(idx_i64, element_size, "byte_offset")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                let elem_ptr = unsafe {
                    builder
                        .build_gep(i8_ptr, &[byte_offset], "elem_ptr")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                };

                let typed_ptr = builder
                    .build_bitcast(
                        elem_ptr,
                        store_val
                            .get_type()
                            .ptr_type(AddressSpace::default()),
                        "typed_ptr",
                    )
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    .into_pointer_value();

                builder
                    .build_store(typed_ptr, store_val)
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
            }
            MemoryOp::Load {
                source,
                index,
                type_ref,
            } => {
                let source_ptr = self
                    .pointers
                    .get(&source.0)
                    .copied()
                    .ok_or_else(|| {
                        CodegenError::UnresolvedNode(format!("M-node pointer: {}", source.0))
                    })?;
                let idx_val = self.resolve_value(&index.0, function, builder)?;

                let i8_ptr = builder
                    .build_bitcast(
                        source_ptr,
                        self.context.i8_type().ptr_type(AddressSpace::default()),
                        "load_base",
                    )
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    .into_pointer_value();

                let element_size = self.context.i64_type().const_int(8, false);
                let idx_i64 = self.int_to_i64(idx_val.into_int_value(), builder)?;
                let byte_offset = builder
                    .build_int_mul(idx_i64, element_size, "byte_offset")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                let elem_ptr = unsafe {
                    builder
                        .build_gep(i8_ptr, &[byte_offset], "elem_ptr")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                };

                let load_type = self
                    .type_ref_to_llvm(type_ref)
                    .unwrap_or(self.context.i64_type().into());
                let typed_ptr = builder
                    .build_bitcast(
                        elem_ptr,
                        load_type.ptr_type(AddressSpace::default()),
                        "typed_load_ptr",
                    )
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                    .into_pointer_value();

                let loaded = builder
                    .build_load(typed_ptr, &node_id.replace(':', "_"))
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                self.values.insert(node_id.to_string(), loaded);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // K-Node emission
    // ------------------------------------------------------------------

    fn emit_control_node(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        // Cycle prevention: if we already emitted this K-node, skip it
        if self.emitted_controls.contains_key(node_id) {
            return Ok(());
        }
        self.emitted_controls.insert(node_id.to_string(), true);

        let control = self
            .control_map
            .get(node_id)
            .ok_or_else(|| CodegenError::UnresolvedNode(node_id.to_string()))?;
        let op = control.op.clone();

        match &op {
            ControlOp::Seq { steps } => {
                for step_ref in steps {
                    let step_id = step_ref.0.clone();
                    let prefix = step_ref.prefix();
                    match prefix {
                        "E" => {
                            self.emit_effect_node(&step_id, function, builder)?;
                        }
                        "K" => {
                            self.emit_control_node(&step_id, function, builder)?;
                        }
                        "C" => {
                            self.emit_compute_side_effect(&step_id, function, builder)?;
                        }
                        "M" => {
                            self.emit_memory_op(&step_id, function, builder)?;
                        }
                        _ => {
                            return Err(CodegenError::Unsupported(format!(
                                "unsupported step type in Seq: {step_id}"
                            )));
                        }
                    }
                }
            }
            ControlOp::Branch {
                condition,
                true_branch,
                false_branch,
            } => {
                let cond_val = self.resolve_value(&condition.0, function, builder)?;
                let cond_int = match cond_val {
                    BasicValueEnum::IntValue(v) => v,
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "branch condition must be integer".to_string(),
                        ));
                    }
                };

                // Ensure the condition is i1 (bool)
                let cond_bool = if cond_int.get_type().get_bit_width() != 1 {
                    builder
                        .build_int_compare(
                            IntPredicate::NE,
                            cond_int,
                            cond_int.get_type().const_int(0, false),
                            "cond_bool",
                        )
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                } else {
                    cond_int
                };

                let then_bb = self.context.append_basic_block(function, "then");
                let else_bb = self.context.append_basic_block(function, "else");
                let merge_bb = self.context.append_basic_block(function, "merge");

                builder
                    .build_conditional_branch(cond_bool, then_bb, else_bb)
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                // Then
                builder.position_at_end(then_bb);
                self.emit_control_node(&true_branch.0, function, builder)?;
                if builder
                    .get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    builder
                        .build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                }

                // Else
                builder.position_at_end(else_bb);
                self.emit_control_node(&false_branch.0, function, builder)?;
                if builder
                    .get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    builder
                        .build_unconditional_branch(merge_bb)
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                }

                builder.position_at_end(merge_bb);
            }
            ControlOp::Par { branches, .. } => {
                // Emit branches sequentially (no real parallelism in codegen)
                for branch in branches {
                    self.emit_control_node(&branch.0, function, builder)?;
                }
            }
            ControlOp::Loop {
                condition,
                body,
                state,
                ..
            } => {
                let header_bb = self.context.append_basic_block(function, "loop_header");
                let body_bb = self.context.append_basic_block(function, "loop_body");
                let exit_bb = self.context.append_basic_block(function, "loop_exit");

                // Initialize state by evaluating it once before the loop
                let _ = self.resolve_value(&state.0, function, builder);

                builder
                    .build_unconditional_branch(header_bb)
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                // Header: evaluate condition, branch
                builder.position_at_end(header_bb);
                let cond_val = self.resolve_value(&condition.0, function, builder)?;
                let cond_int = match cond_val {
                    BasicValueEnum::IntValue(v) => v,
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "loop condition must be integer".to_string(),
                        ));
                    }
                };
                let cond_bool = if cond_int.get_type().get_bit_width() != 1 {
                    builder
                        .build_int_compare(
                            IntPredicate::NE,
                            cond_int,
                            cond_int.get_type().const_int(0, false),
                            "loop_cond",
                        )
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                } else {
                    cond_int
                };

                builder
                    .build_conditional_branch(cond_bool, body_bb, exit_bb)
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                // Body
                builder.position_at_end(body_bb);
                self.emit_control_node(&body.0, function, builder)?;
                if builder
                    .get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    builder
                        .build_unconditional_branch(header_bb)
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                }

                // Exit
                builder.position_at_end(exit_bb);
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Compute node side-effect emission (for Seq steps referencing C:)
    // ------------------------------------------------------------------

    fn emit_compute_side_effect(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if let Some(compute) = self.compute_map.get(node_id) {
            let op = compute.op.clone();
            match &op {
                ComputeOp::AtomicStore {
                    target,
                    value,
                    order,
                } => {
                    self.emit_atomic_store(&target.0, &value.0, order, function, builder)?;
                    self.values.insert(
                        node_id.to_string(),
                        self.context.i64_type().const_int(0, false).into(),
                    );
                }
                ComputeOp::AtomicCas {
                    target,
                    expected,
                    desired,
                    order,
                    success,
                    failure,
                } => {
                    let result = self.emit_atomic_cas(
                        node_id,
                        &target.0,
                        &expected.0,
                        &desired.0,
                        order,
                        function,
                        builder,
                    )?;

                    let loaded = builder
                        .build_extract_value(result, 0, "cas_loaded")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    self.values.insert(node_id.to_string(), loaded);

                    let success_flag = builder
                        .build_extract_value(result, 1, "cas_success")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
                        .into_int_value();

                    let then_bb = self.context.append_basic_block(function, "cas_ok");
                    let else_bb = self.context.append_basic_block(function, "cas_fail");
                    let merge_bb = self.context.append_basic_block(function, "cas_merge");

                    builder
                        .build_conditional_branch(success_flag, then_bb, else_bb)
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

                    builder.position_at_end(then_bb);
                    self.emit_control_node(&success.0, function, builder)?;
                    if builder
                        .get_insert_block()
                        .and_then(|b| b.get_terminator())
                        .is_none()
                    {
                        builder
                            .build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    }

                    builder.position_at_end(else_bb);
                    self.emit_control_node(&failure.0, function, builder)?;
                    if builder
                        .get_insert_block()
                        .and_then(|b| b.get_terminator())
                        .is_none()
                    {
                        builder
                            .build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    }

                    builder.position_at_end(merge_bb);
                }
                _ => {
                    let _ = self.resolve_value(node_id, function, builder)?;
                }
            }
        } else {
            return Err(CodegenError::UnresolvedNode(node_id.to_string()));
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Atomic operations
    // ------------------------------------------------------------------

    fn memory_order_to_llvm(order: &MemoryOrder) -> AtomicOrdering {
        match order {
            MemoryOrder::SeqCst => AtomicOrdering::SequentiallyConsistent,
            MemoryOrder::AcquireRelease => AtomicOrdering::AcquireRelease,
            MemoryOrder::Acquire => AtomicOrdering::Acquire,
            MemoryOrder::Release => AtomicOrdering::Release,
            MemoryOrder::Relaxed => AtomicOrdering::Monotonic,
        }
    }

    fn memory_order_for_load(order: &MemoryOrder) -> AtomicOrdering {
        match order {
            MemoryOrder::SeqCst => AtomicOrdering::SequentiallyConsistent,
            MemoryOrder::AcquireRelease | MemoryOrder::Acquire => AtomicOrdering::Acquire,
            MemoryOrder::Release => AtomicOrdering::Monotonic,
            MemoryOrder::Relaxed => AtomicOrdering::Monotonic,
        }
    }

    fn memory_order_for_store(order: &MemoryOrder) -> AtomicOrdering {
        match order {
            MemoryOrder::SeqCst => AtomicOrdering::SequentiallyConsistent,
            MemoryOrder::AcquireRelease | MemoryOrder::Release => AtomicOrdering::Release,
            MemoryOrder::Acquire => AtomicOrdering::Monotonic,
            MemoryOrder::Relaxed => AtomicOrdering::Monotonic,
        }
    }

    fn emit_atomic_load(
        &mut self,
        node_id: &str,
        source_id: &str,
        order: &MemoryOrder,
        type_ref: &TypeRef,
        _function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let source_ptr = self
            .pointers
            .get(source_id)
            .copied()
            .ok_or_else(|| {
                CodegenError::UnresolvedNode(format!("atomic load source: {source_id}"))
            })?;

        let load_type = self
            .type_ref_to_llvm(type_ref)
            .unwrap_or(self.context.i64_type().into());

        let typed_ptr = builder
            .build_bitcast(
                source_ptr,
                load_type.ptr_type(AddressSpace::default()),
                "atomic_load_ptr",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            .into_pointer_value();

        let ordering = Self::memory_order_for_load(order);
        let loaded = builder
            .build_load(typed_ptr, &node_id.replace(':', "_"))
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        let inst = loaded
            .as_instruction_value()
            .ok_or_else(|| CodegenError::EmitFailed("load not an instruction".to_string()))?;
        inst.set_atomic_ordering(ordering)
            .map_err(|e: &str| CodegenError::EmitFailed(e.to_string()))?;

        self.values.insert(node_id.to_string(), loaded);
        Ok(loaded)
    }

    fn emit_atomic_store(
        &mut self,
        target_id: &str,
        value_id: &str,
        order: &MemoryOrder,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        let target_ptr = self
            .pointers
            .get(target_id)
            .copied()
            .ok_or_else(|| {
                CodegenError::UnresolvedNode(format!("atomic store target: {target_id}"))
            })?;

        let store_val = self.resolve_value(value_id, function, builder)?;

        let typed_ptr = builder
            .build_bitcast(
                target_ptr,
                store_val.get_type().ptr_type(AddressSpace::default()),
                "atomic_store_ptr",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            .into_pointer_value();

        let ordering = Self::memory_order_for_store(order);
        let store_inst = builder
            .build_store(typed_ptr, store_val)
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        store_inst
            .set_atomic_ordering(ordering)
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_atomic_cas(
        &mut self,
        _node_id: &str,
        target_id: &str,
        expected_id: &str,
        desired_id: &str,
        order: &MemoryOrder,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<inkwell::values::StructValue<'ctx>, CodegenError> {
        let target_ptr = self
            .pointers
            .get(target_id)
            .copied()
            .ok_or_else(|| {
                CodegenError::UnresolvedNode(format!("atomic cas target: {target_id}"))
            })?;

        let expected_val = self.resolve_value(expected_id, function, builder)?;
        let desired_val = self.resolve_value(desired_id, function, builder)?;

        let typed_ptr = builder
            .build_bitcast(
                target_ptr,
                expected_val.get_type().ptr_type(AddressSpace::default()),
                "cas_ptr",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            .into_pointer_value();

        let success_ordering = Self::memory_order_to_llvm(order);
        let failure_ordering = match success_ordering {
            AtomicOrdering::SequentiallyConsistent => AtomicOrdering::SequentiallyConsistent,
            AtomicOrdering::AcquireRelease => AtomicOrdering::Acquire,
            AtomicOrdering::Release => AtomicOrdering::Monotonic,
            AtomicOrdering::Acquire => AtomicOrdering::Acquire,
            other => other,
        };

        let result = builder
            .build_cmpxchg(
                typed_ptr,
                expected_val,
                desired_val,
                success_ordering,
                failure_ordering,
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        Ok(result)
    }

    // ------------------------------------------------------------------
    // E-Node emission
    // ------------------------------------------------------------------

    fn emit_effect_node(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        let effect = self
            .effect_map
            .get(node_id)
            .ok_or_else(|| CodegenError::UnresolvedNode(node_id.to_string()))?;
        let op = effect.op.clone();

        match &op {
            EffectOp::Syscall {
                name,
                inputs,
                success,
                ..
            } => {
                match name.as_str() {
                    "write" | "syscall_write" => {
                        self.emit_syscall_write(node_id, inputs, function, builder)?;
                    }
                    "exit" | "syscall_exit" => {
                        self.emit_syscall_exit(inputs, function, builder)?;
                    }
                    "read" | "syscall_read" => {
                        self.emit_syscall_rw(node_id, "read", inputs, function, builder)?;
                    }
                    "open" | "syscall_open" => {
                        self.emit_syscall_open(node_id, inputs, function, builder)?;
                    }
                    "close" | "syscall_close" => {
                        self.emit_syscall_close(node_id, inputs, function, builder)?;
                    }
                    "ioctl" | "syscall_ioctl" => {
                        self.emit_syscall_ioctl(node_id, inputs, function, builder)?;
                    }
                    "nanosleep" | "syscall_nanosleep" => {
                        self.emit_syscall_nanosleep(node_id, inputs, function, builder)?;
                    }
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "unsupported syscall: {name}"
                        )));
                    }
                }

                if let Some(succ) = success {
                    self.emit_control_node(&succ.0, function, builder)?;
                }
            }
            EffectOp::CallExtern {
                target,
                inputs,
                success,
                ..
            } => {
                self.emit_call_extern(node_id, &target.0, inputs, function, builder)?;
                self.emit_control_node(&success.0, function, builder)?;
            }
            EffectOp::Generic { name, .. } => {
                return Err(CodegenError::Unsupported(format!(
                    "generic effect not implemented: {name}"
                )));
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: write(fd, buf, len)
    // ------------------------------------------------------------------

    fn emit_syscall_write(
        &mut self,
        node_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.len() != 3 {
            return Err(CodegenError::Unsupported(
                "write syscall expects 3 inputs (fd, buf, len)".to_string(),
            ));
        }

        let fd_val = self.resolve_value(&inputs[0].0, function, builder)?;
        let buf_val = self.resolve_pointer(&inputs[1].0, function, builder)?;
        let len_val = self.resolve_value(&inputs[2].0, function, builder)?;

        let write_fn = *self.functions.get("write").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function 'write' not declared".to_string())
        })?;

        let fd_i32 = match fd_val {
            BasicValueEnum::IntValue(v) => {
                builder
                    .build_int_truncate(v, self.context.i32_type(), "fd_i32")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            }
            _ => {
                return Err(CodegenError::Unsupported(
                    "fd must be integer".to_string(),
                ));
            }
        };

        let result = builder
            .build_call(
                write_fn,
                &[fd_i32.into(), buf_val.into(), len_val.into()],
                "write_result",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: exit(code)
    // ------------------------------------------------------------------

    fn emit_syscall_exit(
        &mut self,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.is_empty() {
            return Err(CodegenError::Unsupported(
                "exit syscall expects 1 input (code)".to_string(),
            ));
        }

        let code_val = self.resolve_value(&inputs[0].0, function, builder)?;

        let exit_fn = *self.functions.get("_exit").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function '_exit' not declared".to_string())
        })?;

        let code_i32 = match code_val {
            BasicValueEnum::IntValue(v) => {
                builder
                    .build_int_truncate(v, self.context.i32_type(), "exit_code")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            }
            _ => {
                return Err(CodegenError::Unsupported(
                    "exit code must be integer".to_string(),
                ));
            }
        };

        builder
            .build_call(exit_fn, &[code_i32.into()], "")
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        builder
            .build_unreachable()
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        let dead_bb = self.context.append_basic_block(function, "after_exit");
        builder.position_at_end(dead_bb);

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: read(fd, buf, len)
    // ------------------------------------------------------------------

    fn emit_syscall_rw(
        &mut self,
        node_id: &str,
        fname: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "{fname} syscall expects 3 inputs (fd, buf, len)"
            )));
        }

        let fd_val = self.resolve_value(&inputs[0].0, function, builder)?;
        let buf_val = self.resolve_pointer(&inputs[1].0, function, builder)?;
        let len_val = self.resolve_value(&inputs[2].0, function, builder)?;

        let fn_val = *self.functions.get(fname).ok_or_else(|| {
            CodegenError::UnresolvedNode(format!("libc function '{fname}' not declared"))
        })?;

        let fd_i32 = self.int_to_i32(fd_val.into_int_value(), builder)?;

        let result = builder
            .build_call(
                fn_val,
                &[fd_i32.into(), buf_val.into(), len_val.into()],
                &format!("{fname}_result"),
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: open(path, flags)
    // ------------------------------------------------------------------

    fn emit_syscall_open(
        &mut self,
        node_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.len() < 2 {
            return Err(CodegenError::Unsupported(
                "open syscall expects at least 2 inputs (path, flags)".to_string(),
            ));
        }

        let path_val = self.resolve_pointer(&inputs[0].0, function, builder)?;
        let flags_val = self.resolve_value(&inputs[1].0, function, builder)?;

        let open_fn = *self.functions.get("open").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function 'open' not declared".to_string())
        })?;

        let flags_i32 = self.int_to_i32(flags_val.into_int_value(), builder)?;

        let result = builder
            .build_call(
                open_fn,
                &[path_val.into(), flags_i32.into()],
                "open_result",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: close(fd)
    // ------------------------------------------------------------------

    fn emit_syscall_close(
        &mut self,
        node_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.is_empty() {
            return Err(CodegenError::Unsupported(
                "close syscall expects 1 input (fd)".to_string(),
            ));
        }

        let fd_val = self.resolve_value(&inputs[0].0, function, builder)?;
        let close_fn = *self.functions.get("close").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function 'close' not declared".to_string())
        })?;

        let fd_i32 = self.int_to_i32(fd_val.into_int_value(), builder)?;

        let result = builder
            .build_call(close_fn, &[fd_i32.into()], "close_result")
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: ioctl(fd, request, ...)
    // ------------------------------------------------------------------

    fn emit_syscall_ioctl(
        &mut self,
        node_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.len() < 2 {
            return Err(CodegenError::Unsupported(
                "ioctl syscall expects at least 2 inputs (fd, request)".to_string(),
            ));
        }

        let fd_val = self.resolve_value(&inputs[0].0, function, builder)?;
        let req_val = self.resolve_value(&inputs[1].0, function, builder)?;

        let ioctl_fn = *self.functions.get("ioctl").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function 'ioctl' not declared".to_string())
        })?;

        let fd_i32 = self.int_to_i32(fd_val.into_int_value(), builder)?;

        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![fd_i32.into(), req_val.into()];

        for input in inputs.iter().skip(2) {
            let val = self.resolve_value(&input.0, function, builder)?;
            args.push(val.into());
        }

        let result = builder
            .build_call(ioctl_fn, &args, "ioctl_result")
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Syscall: nanosleep(req, rem)
    // ------------------------------------------------------------------

    fn emit_syscall_nanosleep(
        &mut self,
        node_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        if inputs.len() < 2 {
            return Err(CodegenError::Unsupported(
                "nanosleep syscall expects 2 inputs (req, rem)".to_string(),
            ));
        }

        let req_val = self.resolve_pointer(&inputs[0].0, function, builder)?;
        let rem_val = self.resolve_pointer(&inputs[1].0, function, builder)?;

        let nanosleep_fn = *self.functions.get("nanosleep").ok_or_else(|| {
            CodegenError::UnresolvedNode("libc function 'nanosleep' not declared".to_string())
        })?;

        let result = builder
            .build_call(
                nanosleep_fn,
                &[req_val.into(), rem_val.into()],
                "nanosleep_result",
            )
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // CallExtern — FFI call to an X-Node declared function
    // ------------------------------------------------------------------

    fn emit_call_extern(
        &mut self,
        node_id: &str,
        target_id: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
        let callee = self
            .functions
            .get(target_id)
            .copied()
            .ok_or_else(|| CodegenError::UnresolvedNode(format!("extern function: {target_id}")))?;

        let param_types: Vec<_> = callee.get_type().get_param_types();

        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        for (idx, input) in inputs.iter().enumerate() {
            let expected_type = param_types.get(idx);
            let prefix = input.prefix();
            match prefix {
                "C" => {
                    if self.pointers.contains_key(&input.0) {
                        let ptr = self.resolve_pointer(&input.0, function, builder)?;
                        if let Some(inkwell::types::BasicTypeEnum::IntType(int_ty)) = expected_type
                        {
                            let int_val = builder
                                .build_ptr_to_int(ptr, *int_ty, "ptr_to_int")
                                .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                            args.push(int_val.into());
                        } else {
                            args.push(ptr.into());
                        }
                    } else {
                        let val = self.resolve_value(&input.0, function, builder)?;
                        args.push(val.into());
                    }
                }
                "E" => {
                    let val = self
                        .values
                        .get(&input.0)
                        .ok_or_else(|| CodegenError::UnresolvedNode(input.0.clone()))?;
                    let cast_val = self.cast_if_needed(*val, expected_type, builder)?;
                    args.push(cast_val.into());
                }
                _ => {
                    let val = self.resolve_value(&input.0, function, builder)?;
                    args.push(val.into());
                }
            }
        }

        let call_name = if callee.get_type().get_return_type().is_some() {
            &format!("call_{}", node_id.replace(':', "_"))
        } else {
            ""
        };

        let result = builder
            .build_call(callee, &args, call_name)
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        if let Some(ret) = result.try_as_basic_value().left() {
            self.values.insert(node_id.to_string(), ret);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Value resolution — turn a NodeRef string into an LLVM value
    // ------------------------------------------------------------------

    fn resolve_value(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        // Already computed?
        if let Some(val) = self.values.get(node_id) {
            return Ok(*val);
        }

        // Is it a C-node constant?
        if let Some(compute) = self.compute_map.get(node_id) {
            let compute_op = compute.op.clone();
            match &compute_op {
                ComputeOp::Const { value, .. } => {
                    let val = self.literal_to_llvm(value);
                    self.values.insert(node_id.to_string(), val);
                    return Ok(val);
                }
                ComputeOp::ConstBytes { .. } => {
                    return Err(CodegenError::Unsupported(format!(
                        "const_bytes {node_id} used as value, use as pointer instead"
                    )));
                }
                ComputeOp::Arith {
                    opcode, inputs, ..
                } => {
                    let lhs = self.resolve_value(&inputs[0].0, function, builder)?;
                    let rhs = self.resolve_value(&inputs[1].0, function, builder)?;
                    let lhs_int = lhs.into_int_value();
                    let rhs_int = rhs.into_int_value();

                    let (lhs_norm, rhs_norm) =
                        self.normalize_int_widths(lhs_int, rhs_int, builder)?;

                    let result = match opcode.as_str() {
                        "add" => builder
                            .build_int_add(lhs_norm, rhs_norm, "add")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "sub" => builder
                            .build_int_sub(lhs_norm, rhs_norm, "sub")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "mul" => builder
                            .build_int_mul(lhs_norm, rhs_norm, "mul")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "div" | "sdiv" => builder
                            .build_int_signed_div(lhs_norm, rhs_norm, "div")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "udiv" => builder
                            .build_int_unsigned_div(lhs_norm, rhs_norm, "udiv")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "mod" | "srem" => builder
                            .build_int_signed_rem(lhs_norm, rhs_norm, "rem")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "urem" => builder
                            .build_int_unsigned_rem(lhs_norm, rhs_norm, "urem")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "and" => builder
                            .build_and(lhs_norm, rhs_norm, "and")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "or" => builder
                            .build_or(lhs_norm, rhs_norm, "or")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "xor" => builder
                            .build_xor(lhs_norm, rhs_norm, "xor")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "shl" => builder
                            .build_left_shift(lhs_norm, rhs_norm, "shl")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "shr" | "ashr" => builder
                            .build_right_shift(lhs_norm, rhs_norm, true, "shr")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "lshr" => builder
                            .build_right_shift(lhs_norm, rhs_norm, false, "lshr")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        // Comparison opcodes producing i1
                        "gt" | "sgt" => builder
                            .build_int_compare(IntPredicate::SGT, lhs_norm, rhs_norm, "cmp_gt")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "lt" | "slt" => builder
                            .build_int_compare(IntPredicate::SLT, lhs_norm, rhs_norm, "cmp_lt")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "gte" | "sge" => builder
                            .build_int_compare(IntPredicate::SGE, lhs_norm, rhs_norm, "cmp_gte")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "lte" | "sle" => builder
                            .build_int_compare(IntPredicate::SLE, lhs_norm, rhs_norm, "cmp_lte")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "eq" => builder
                            .build_int_compare(IntPredicate::EQ, lhs_norm, rhs_norm, "cmp_eq")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "neq" => builder
                            .build_int_compare(IntPredicate::NE, lhs_norm, rhs_norm, "cmp_neq")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "ugt" => builder
                            .build_int_compare(IntPredicate::UGT, lhs_norm, rhs_norm, "cmp_ugt")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "ult" => builder
                            .build_int_compare(IntPredicate::ULT, lhs_norm, rhs_norm, "cmp_ult")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "uge" => builder
                            .build_int_compare(IntPredicate::UGE, lhs_norm, rhs_norm, "cmp_uge")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "ule" => builder
                            .build_int_compare(IntPredicate::ULE, lhs_norm, rhs_norm, "cmp_ule")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        _ => {
                            return Err(CodegenError::Unsupported(format!(
                                "unsupported arith opcode: {opcode}"
                            )));
                        }
                    };
                    let val = BasicValueEnum::IntValue(result);
                    self.values.insert(node_id.to_string(), val);
                    return Ok(val);
                }
                ComputeOp::Generic {
                    name, inputs, ..
                } => {
                    let val = self.emit_generic_compute(node_id, name, inputs, function, builder)?;
                    self.values.insert(node_id.to_string(), val);
                    return Ok(val);
                }
                ComputeOp::AtomicLoad {
                    source,
                    order,
                    type_ref,
                } => {
                    return self.emit_atomic_load(
                        node_id, &source.0, order, type_ref, function, builder,
                    );
                }
                ComputeOp::AtomicStore {
                    target,
                    value,
                    order,
                } => {
                    self.emit_atomic_store(&target.0, &value.0, order, function, builder)?;
                    let val = self.context.i64_type().const_int(0, false).into();
                    self.values.insert(node_id.to_string(), val);
                    return Ok(val);
                }
                ComputeOp::AtomicCas {
                    target,
                    expected,
                    desired,
                    order,
                    ..
                } => {
                    let cas_result = self.emit_atomic_cas(
                        node_id, &target.0, &expected.0, &desired.0, order, function, builder,
                    )?;
                    let loaded = builder
                        .build_extract_value(cas_result, 0, "cas_loaded")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    self.values.insert(node_id.to_string(), loaded);
                    return Ok(loaded);
                }
                ComputeOp::CallPure {
                    target, inputs, ..
                } => {
                    let val =
                        self.emit_generic_compute(node_id, target, inputs, function, builder)?;
                    self.values.insert(node_id.to_string(), val);
                    return Ok(val);
                }
            }
        }

        Err(CodegenError::UnresolvedNode(node_id.to_string()))
    }

    // ------------------------------------------------------------------
    // Generic compute: comparison ops, etc.
    // ------------------------------------------------------------------

    fn emit_generic_compute(
        &mut self,
        _node_id: &str,
        name: &str,
        inputs: &[NodeRef],
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match name {
            "gt" | "lt" | "gte" | "lte" | "eq" | "neq" | "sgt" | "slt" | "sge" | "sle" => {
                if inputs.len() < 2 {
                    return Err(CodegenError::Unsupported(format!(
                        "comparison {name} expects 2 inputs"
                    )));
                }
                let lhs = self.resolve_value(&inputs[0].0, function, builder)?;
                let rhs = self.resolve_value(&inputs[1].0, function, builder)?;
                let lhs_int = lhs.into_int_value();
                let rhs_int = rhs.into_int_value();

                let (lhs_norm, rhs_norm) =
                    self.normalize_int_widths(lhs_int, rhs_int, builder)?;

                let pred = match name {
                    "gt" | "sgt" => IntPredicate::SGT,
                    "lt" | "slt" => IntPredicate::SLT,
                    "gte" | "sge" => IntPredicate::SGE,
                    "lte" | "sle" => IntPredicate::SLE,
                    "eq" => IntPredicate::EQ,
                    "neq" => IntPredicate::NE,
                    _ => IntPredicate::EQ,
                };

                let cmp = builder
                    .build_int_compare(pred, lhs_norm, rhs_norm, &format!("cmp_{name}"))
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(BasicValueEnum::IntValue(cmp))
            }
            "not" => {
                if inputs.is_empty() {
                    return Err(CodegenError::Unsupported("not expects 1 input".to_string()));
                }
                let val = self.resolve_value(&inputs[0].0, function, builder)?;
                let int_val = val.into_int_value();
                let result = builder
                    .build_not(int_val, "not")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(BasicValueEnum::IntValue(result))
            }
            "neg" => {
                if inputs.is_empty() {
                    return Err(CodegenError::Unsupported("neg expects 1 input".to_string()));
                }
                let val = self.resolve_value(&inputs[0].0, function, builder)?;
                let int_val = val.into_int_value();
                let result = builder
                    .build_int_neg(int_val, "neg")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(BasicValueEnum::IntValue(result))
            }
            _ => {
                // Unknown generic compute: return a zero constant
                Ok(self.context.i64_type().const_int(0, false).into())
            }
        }
    }

    // ------------------------------------------------------------------
    // Integer width helpers
    // ------------------------------------------------------------------

    fn normalize_int_widths(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let lw = lhs.get_type().get_bit_width();
        let rw = rhs.get_type().get_bit_width();
        if lw == rw {
            return Ok((lhs, rhs));
        }
        if lw > rw {
            let extended = builder
                .build_int_z_extend(rhs, lhs.get_type(), "widen_rhs")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
            Ok((lhs, extended))
        } else {
            let extended = builder
                .build_int_z_extend(lhs, rhs.get_type(), "widen_lhs")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
            Ok((extended, rhs))
        }
    }

    fn int_to_i32(
        &self,
        val: IntValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let width = val.get_type().get_bit_width();
        if width == 32 {
            Ok(val)
        } else if width > 32 {
            builder
                .build_int_truncate(val, self.context.i32_type(), "to_i32")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))
        } else {
            builder
                .build_int_z_extend(val, self.context.i32_type(), "to_i32")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))
        }
    }

    fn int_to_i64(
        &self,
        val: IntValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let width = val.get_type().get_bit_width();
        if width == 64 {
            Ok(val)
        } else if width > 64 {
            builder
                .build_int_truncate(val, self.context.i64_type(), "to_i64")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))
        } else {
            builder
                .build_int_z_extend(val, self.context.i64_type(), "to_i64")
                .map_err(|e| CodegenError::EmitFailed(e.to_string()))
        }
    }

    fn cast_if_needed(
        &self,
        val: BasicValueEnum<'ctx>,
        expected: Option<&inkwell::types::BasicTypeEnum<'ctx>>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let Some(expected_ty) = expected else {
            return Ok(val);
        };
        match (val, expected_ty) {
            (BasicValueEnum::IntValue(iv), inkwell::types::BasicTypeEnum::PointerType(pt)) => {
                let ptr = builder
                    .build_int_to_ptr(iv, *pt, "int_to_ptr")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(ptr.into())
            }
            (BasicValueEnum::PointerValue(pv), inkwell::types::BasicTypeEnum::IntType(it)) => {
                let int_val = builder
                    .build_ptr_to_int(pv, *it, "ptr_to_int")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(int_val.into())
            }
            (BasicValueEnum::IntValue(iv), inkwell::types::BasicTypeEnum::IntType(it)) => {
                let src_width = iv.get_type().get_bit_width();
                let dst_width = it.get_bit_width();
                if src_width == dst_width {
                    Ok(val)
                } else if src_width < dst_width {
                    let extended = builder
                        .build_int_z_extend(iv, *it, "widen")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    Ok(extended.into())
                } else {
                    let truncated = builder
                        .build_int_truncate(iv, *it, "narrow")
                        .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                    Ok(truncated.into())
                }
            }
            _ => Ok(val),
        }
    }

    fn resolve_pointer(
        &self,
        node_id: &str,
        _function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(global_ptr) = self.pointers.get(node_id) {
            let i32_type = self.context.i32_type();
            let zero = i32_type.const_int(0, false);
            let ptr = unsafe {
                builder
                    .build_gep(*global_ptr, &[zero, zero], "buf_ptr")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?
            };
            return Ok(ptr);
        }

        Err(CodegenError::UnresolvedNode(format!(
            "pointer for {node_id}"
        )))
    }

    // ------------------------------------------------------------------
    // Literal -> LLVM constant
    // ------------------------------------------------------------------

    fn literal_to_llvm(&self, lit: &Literal) -> BasicValueEnum<'ctx> {
        match lit {
            Literal::Integer { value } => self
                .context
                .i64_type()
                .const_int(*value as u64, *value < 0)
                .into(),
            Literal::Float { value } => self.context.f64_type().const_float(*value).into(),
            Literal::Bool { value } => self
                .context
                .bool_type()
                .const_int(*value as u64, false)
                .into(),
            Literal::Str { value } => {
                self.context
                    .i64_type()
                    .const_int(value.len() as u64, false)
                    .into()
            }
        }
    }

    // ------------------------------------------------------------------
    // Finalize: produce IR text and optional object file
    // ------------------------------------------------------------------

    fn finish(&self) -> Result<CodegenResult, CodegenError> {
        let llvm_ir = self.module.print_to_string().to_string();

        let output_bytes = match self.config.output_format {
            OutputFormat::LlvmIr => llvm_ir.as_bytes().to_vec(),
            OutputFormat::ObjectFile | OutputFormat::Assembly => {
                self.emit_machine_code()?
            }
        };

        Ok(CodegenResult {
            llvm_ir,
            output_bytes,
        })
    }

    fn emit_machine_code(&self) -> Result<Vec<u8>, CodegenError> {
        Target::initialize_x86(&InitializationConfig::default());

        let triple = TargetTriple::create(&self.config.target_triple);
        let target = Target::from_triple(&triple)
            .map_err(|e| CodegenError::TargetInitFailed(e.to_string()))?;

        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                self.config.opt_level.to_inkwell(),
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| {
                CodegenError::TargetInitFailed("failed to create target machine".to_string())
            })?;

        let file_type = match self.config.output_format {
            OutputFormat::Assembly => FileType::Assembly,
            _ => FileType::Object,
        };

        let buf = machine
            .write_to_memory_buffer(&self.module, file_type)
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        Ok(buf.as_slice().to_vec())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_ftl;

    fn parse_hello_world() -> Program {
        let source = std::fs::read_to_string("testdata/hello_world.ftl")
            .expect("failed to read hello_world.ftl");
        let result = parse_ftl(&source);
        result.ast.expect("failed to parse hello_world.ftl")
    }

    #[test]
    fn hello_world_generates_ir() {
        let program = parse_hello_world();
        let config = CodegenConfig {
            output_format: OutputFormat::LlvmIr,
            ..CodegenConfig::default()
        };
        let result = codegen(&program, &config).expect("codegen failed");
        assert!(!result.llvm_ir.is_empty());
        assert!(result.llvm_ir.contains("define i32 @main"));
        assert!(result.llvm_ir.contains("flux_module"));
    }

    #[test]
    fn hello_world_ir_contains_global_bytes() {
        let program = parse_hello_world();
        let config = CodegenConfig {
            output_format: OutputFormat::LlvmIr,
            ..CodegenConfig::default()
        };
        let result = codegen(&program, &config).expect("codegen failed");
        assert!(result.llvm_ir.contains("const_C_c1"));
        assert!(result.llvm_ir.contains("[12 x i8]"));
    }

    #[test]
    fn hello_world_ir_contains_write_call() {
        let program = parse_hello_world();
        let config = CodegenConfig {
            output_format: OutputFormat::LlvmIr,
            ..CodegenConfig::default()
        };
        let result = codegen(&program, &config).expect("codegen failed");
        assert!(result.llvm_ir.contains("call"));
        assert!(result.llvm_ir.contains("@write"));
    }

    #[test]
    fn hello_world_ir_contains_exit_call() {
        let program = parse_hello_world();
        let config = CodegenConfig {
            output_format: OutputFormat::LlvmIr,
            ..CodegenConfig::default()
        };
        let result = codegen(&program, &config).expect("codegen failed");
        assert!(result.llvm_ir.contains("@_exit"));
    }
}
