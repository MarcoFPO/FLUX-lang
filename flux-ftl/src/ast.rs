use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// NodeRef — typed reference to any node in the graph ("T:a1", "C:c1", etc.)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef(pub String);

impl NodeRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn prefix(&self) -> &str {
        self.0.split(':').next().unwrap_or("")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// TypeRef — either a node reference (T:a1) or a builtin name ("unit")
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeRef {
    Id { node: NodeRef },
    Builtin { name: String },
}

// ---------------------------------------------------------------------------
// Literal values
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Literal {
    Integer { value: i64 },
    Float { value: f64 },
    Bool { value: bool },
    Str { value: String },
}

// ---------------------------------------------------------------------------
// Layout — memory layout strategy for struct types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Optimal,
    Packed,
    CAbi,
}

impl Default for Layout {
    fn default() -> Self {
        Self::Optimal
    }
}

// ---------------------------------------------------------------------------
// MemoryOrder — atomics ordering semantics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrder {
    SeqCst,
    AcquireRelease,
    Acquire,
    Release,
    Relaxed,
}

// ---------------------------------------------------------------------------
// SyncMode — parallel branch synchronisation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Barrier,
    None,
}

// ---------------------------------------------------------------------------
// T-Node — Type definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    pub id: NodeRef,
    pub body: TypeBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    pub type_ref: TypeRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantCase {
    pub tag: String,
    pub payload: TypeRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeBody {
    Integer {
        bits: u32,
        signed: bool,
    },
    Float {
        bits: u32,
    },
    Boolean,
    Unit,
    Struct {
        fields: Vec<StructField>,
        #[serde(default)]
        layout: Layout,
    },
    Array {
        element: TypeRef,
        max_length: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        constraint: Option<Formula>,
    },
    Variant {
        cases: Vec<VariantCase>,
    },
    Fn {
        params: Vec<TypeRef>,
        result: Box<TypeRef>,
        effects: Vec<String>,
    },
    Opaque {
        size: u32,
        align: u8,
    },
}

// ---------------------------------------------------------------------------
// R-Node — Region definitions (memory lifetimes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionDef {
    pub id: NodeRef,
    pub lifetime: Lifetime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    Static,
    Scoped,
}

// ---------------------------------------------------------------------------
// C-Node — Compute definitions (pure computations)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeDef {
    pub id: NodeRef,
    pub op: ComputeOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComputeOp {
    Const {
        value: Literal,
        type_ref: TypeRef,
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<NodeRef>,
    },
    ConstBytes {
        value: Vec<u8>,
        type_ref: TypeRef,
        region: NodeRef,
    },
    Arith {
        opcode: String,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
    },
    CallPure {
        target: String,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
    },
    Generic {
        name: String,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<NodeRef>,
    },
    AtomicLoad {
        source: NodeRef,
        order: MemoryOrder,
        type_ref: TypeRef,
    },
    AtomicStore {
        target: NodeRef,
        value: NodeRef,
        order: MemoryOrder,
    },
    AtomicCas {
        target: NodeRef,
        expected: NodeRef,
        desired: NodeRef,
        order: MemoryOrder,
        success: NodeRef,
        failure: NodeRef,
    },
}

// ---------------------------------------------------------------------------
// E-Node — Effect definitions (side effects: IO, syscalls, FFI calls)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDef {
    pub id: NodeRef,
    pub op: EffectOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectOp {
    Syscall {
        name: String,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
        effects: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<NodeRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<NodeRef>,
    },
    CallExtern {
        target: NodeRef,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
        effects: Vec<String>,
        success: NodeRef,
        failure: NodeRef,
    },
    Generic {
        name: String,
        inputs: Vec<NodeRef>,
        type_ref: TypeRef,
        effects: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<NodeRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<NodeRef>,
    },
}

// ---------------------------------------------------------------------------
// K-Node — Control flow definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlDef {
    pub id: NodeRef,
    pub op: ControlOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlOp {
    Seq {
        steps: Vec<NodeRef>,
    },
    Branch {
        condition: NodeRef,
        true_branch: NodeRef,
        false_branch: NodeRef,
    },
    Loop {
        condition: NodeRef,
        body: NodeRef,
        state: NodeRef,
        state_type: TypeRef,
    },
    Par {
        branches: Vec<NodeRef>,
        sync: SyncMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        memory_order: Option<MemoryOrder>,
    },
}

// ---------------------------------------------------------------------------
// V-Node — Contract definitions (verification obligations)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDef {
    pub id: NodeRef,
    pub target: NodeRef,
    pub clauses: Vec<ContractClause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<TrustLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContractClause {
    Pre { formula: Formula },
    Post { formula: Formula },
    Invariant { formula: Formula },
    Assume { formula: Formula },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Proven,
    Extern,
}

// ---------------------------------------------------------------------------
// Formula — SMT-LIB2-compatible contract language
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Formula {
    And {
        left: Box<Formula>,
        right: Box<Formula>,
    },
    Or {
        left: Box<Formula>,
        right: Box<Formula>,
    },
    Not {
        inner: Box<Formula>,
    },
    Comparison {
        left: Expr,
        op: CmpOp,
        right: Expr,
    },
    Forall {
        var: String,
        range_start: Expr,
        range_end: Expr,
        body: Box<Formula>,
    },
    BoolLit {
        value: bool,
    },
    FieldAccess {
        node: NodeRef,
        fields: Vec<String>,
    },
    PredicateCall {
        name: String,
        args: Vec<Formula>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}

// ---------------------------------------------------------------------------
// Expr — arithmetic / value expressions inside formulas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    IntLit { value: i64 },
    FloatLit { value: f64 },
    Ident { name: String },
    FieldAccess { node: NodeRef, fields: Vec<String> },
    BinOp { left: Box<Expr>, op: ArithBinOp, right: Box<Expr> },
    Result,
    State,
    PredicateCall {
        name: String,
        args: Vec<Expr>,
    },
    EmptySet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArithBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

// ---------------------------------------------------------------------------
// M-Node — Memory operations (region-bound)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDef {
    pub id: NodeRef,
    pub op: MemoryOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryOp {
    Alloc {
        type_ref: TypeRef,
        region: NodeRef,
    },
    Load {
        source: NodeRef,
        index: NodeRef,
        type_ref: TypeRef,
    },
    Store {
        target: NodeRef,
        index: NodeRef,
        value: NodeRef,
    },
}

// ---------------------------------------------------------------------------
// X-Node — Extern (FFI) declarations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternDef {
    pub id: NodeRef,
    pub name: String,
    pub abi: Abi,
    pub params: Vec<TypeRef>,
    pub result: TypeRef,
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Abi {
    C,
    SystemV,
    Aapcs64,
}

// ---------------------------------------------------------------------------
// Program — top-level container for the entire FTL graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub types: Vec<TypeDef>,
    pub regions: Vec<RegionDef>,
    pub computes: Vec<ComputeDef>,
    pub effects: Vec<EffectDef>,
    pub controls: Vec<ControlDef>,
    pub contracts: Vec<ContractDef>,
    pub memories: Vec<MemoryDef>,
    pub externs: Vec<ExternDef>,
    pub entry: NodeRef,
}
