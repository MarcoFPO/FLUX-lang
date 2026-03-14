// ---------------------------------------------------------------------------
// codegen.rs — LLVM IR generation from FTL AST via inkwell
// ---------------------------------------------------------------------------
//
// Translates a parsed FTL Program into LLVM IR using inkwell (LLVM 14).
// Phase 1 focuses on:
//   - Const / ConstBytes C-Nodes
//   - Syscall E-Nodes (write, exit)
//   - CallExtern E-Nodes (FFI calls)
//   - Seq K-Nodes (sequential control flow)
//   - Extern X-Node declarations
//
// The generated IR links against libc for write() and _exit().
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
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
    /// Unsupported AST construct for Phase 1.
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
    #[allow(dead_code)]
    extern_map: HashMap<String, &'prog ExternDef>,

    // LLVM values produced during emission
    values: HashMap<String, BasicValueEnum<'ctx>>,
    // Pointers for const_bytes globals
    pointers: HashMap<String, PointerValue<'ctx>>,
    // Declared libc / extern functions
    functions: HashMap<String, FunctionValue<'ctx>>,
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
            extern_map,
            values: HashMap::new(),
            pointers: HashMap::new(),
            functions: HashMap::new(),
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

                // Store pointer to first element (will be used when referencing this node)
                // We store None for now; the pointer is computed in-function via GEP
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
    // K-Node emission
    // ------------------------------------------------------------------

    fn emit_control_node(
        &mut self,
        node_id: &str,
        function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<(), CodegenError> {
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

                let then_bb = self.context.append_basic_block(function, "then");
                let else_bb = self.context.append_basic_block(function, "else");
                let merge_bb = self.context.append_basic_block(function, "merge");

                builder
                    .build_conditional_branch(cond_int, then_bb, else_bb)
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
                // Phase 1: emit branches sequentially
                for branch in branches {
                    self.emit_control_node(&branch.0, function, builder)?;
                }
            }
            ControlOp::Loop { .. } => {
                return Err(CodegenError::Unsupported(
                    "Loop K-nodes not yet implemented".to_string(),
                ));
            }
        }

        Ok(())
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
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "unsupported syscall: {name}"
                        )));
                    }
                }

                // Follow the success continuation if present
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

                // Follow the success continuation
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

        // fd needs to be i32
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

        // code needs to be i32
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

        // _exit never returns, mark as unreachable
        builder
            .build_unreachable()
            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;

        // Create a new block for any code after (it won't be reached but
        // allows the builder to continue without errors for subsequent steps)
        let dead_bb = self.context.append_basic_block(function, "after_exit");
        builder.position_at_end(dead_bb);

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

        let param_types: Vec<_> = callee
            .get_type()
            .get_param_types();

        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        for (idx, input) in inputs.iter().enumerate() {
            let expected_type = param_types.get(idx);
            let prefix = input.prefix();
            match prefix {
                "C" => {
                    if self.pointers.contains_key(&input.0) {
                        let ptr = self.resolve_pointer(&input.0, function, builder)?;
                        // If the function expects an integer (e.g. T:ptr = i64), cast pointer to int
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
                    // Cast if needed: value is int but param expects pointer, or vice versa
                    let cast_val =
                        self.cast_if_needed(*val, expected_type, builder)?;
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
        _function: FunctionValue<'ctx>,
        _builder: &inkwell::builder::Builder<'ctx>,
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
                    // For const_bytes used as a value (not pointer), return the length
                    // This shouldn't normally happen; const_bytes are used via pointers
                    return Err(CodegenError::Unsupported(format!(
                        "const_bytes {node_id} used as value, use as pointer instead"
                    )));
                }
                ComputeOp::Arith {
                    opcode, inputs, ..
                } => {
                    let lhs = self.resolve_value(&inputs[0].0, _function, _builder)?;
                    let rhs = self.resolve_value(&inputs[1].0, _function, _builder)?;
                    let lhs_int = lhs.into_int_value();
                    let rhs_int = rhs.into_int_value();
                    let result = match opcode.as_str() {
                        "add" => _builder
                            .build_int_add(lhs_int, rhs_int, "add")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "sub" => _builder
                            .build_int_sub(lhs_int, rhs_int, "sub")
                            .map_err(|e| CodegenError::EmitFailed(e.to_string()))?,
                        "mul" => _builder
                            .build_int_mul(lhs_int, rhs_int, "mul")
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
                _ => {
                    return Err(CodegenError::Unsupported(format!(
                        "unsupported compute op for {node_id}"
                    )));
                }
            }
        }

        Err(CodegenError::UnresolvedNode(node_id.to_string()))
    }

    /// Cast a value to match the expected parameter type if there is a mismatch.
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
            // int -> ptr
            (BasicValueEnum::IntValue(iv), inkwell::types::BasicTypeEnum::PointerType(pt)) => {
                let ptr = builder
                    .build_int_to_ptr(iv, *pt, "int_to_ptr")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(ptr.into())
            }
            // ptr -> int
            (BasicValueEnum::PointerValue(pv), inkwell::types::BasicTypeEnum::IntType(it)) => {
                let int_val = builder
                    .build_ptr_to_int(pv, *it, "ptr_to_int")
                    .map_err(|e| CodegenError::EmitFailed(e.to_string()))?;
                Ok(int_val.into())
            }
            _ => Ok(val),
        }
    }

    /// Resolve a NodeRef to a pointer value (used for const_bytes buffers).
    fn resolve_pointer(
        &self,
        node_id: &str,
        _function: FunctionValue<'ctx>,
        builder: &inkwell::builder::Builder<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(global_ptr) = self.pointers.get(node_id) {
            // GEP to get pointer to first element of the global array
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
    // Literal → LLVM constant
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
                // Strings become global constants; return pointer
                // For now just return the length as an integer
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
        // Should contain "Hello World\n" as byte constants
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
