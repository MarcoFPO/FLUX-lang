# FTL (FLUX Text Language) Specification

Version: v3
Parser: PEG grammar (pest)
File extension: `.ftl`

## Program Structure

An FTL program is a sequence of node definitions and one `entry` declaration.
Nodes are defined in any order. Forward references are allowed.

```
<statement>*
entry: <node_ref>
```

## Node ID Format

Every node has a typed ID: `<prefix>:<identifier>`

| Prefix | Node Type | Purpose |
|--------|-----------|---------|
| T | Type | Type definitions |
| R | Region | Memory lifetime regions |
| C | Compute | Pure computations |
| E | Effect | Side effects (IO, syscalls, FFI) |
| K | Control | Control flow (seq, branch, loop, par) |
| V | Contract | Verification obligations |
| M | Memory | Memory operations (alloc, load, store) |
| X | Extern | FFI declarations |

Identifier: `[a-zA-Z0-9_]+`
Examples: `T:a1`, `C:c_hello`, `K:f_main`, `X:ext1`

## Comments

```
// single line comment
```

## T-Node (Type Definitions)

```
T:<id> = integer { bits: <8|16|32|64|128>, signed: <true|false> }
T:<id> = float { bits: <32|64> }
T:<id> = boolean
T:<id> = unit
T:<id> = array { element: <type_ref>, max_length: <int> }
T:<id> = array { element: <type_ref>, max_length: <int>, constraint: <formula> }
T:<id> = struct { fields: [<name>: <type_ref>, ...] }
T:<id> = struct { fields: [<name>: <type_ref>, ...], layout: <PACKED|C_ABI|OPTIMAL> }
T:<id> = variant { cases: [<TAG>: <type_ref>, ...] }
T:<id> = fn { params: [<type_ref>, ...], result: <type_ref> }
T:<id> = fn { params: [<type_ref>, ...], result: <type_ref>, effects: [<effect>, ...] }
T:<id> = opaque { size: <int>, align: <int> }
```

### Type References

A `<type_ref>` is either a T-node ID or a builtin type name.

Builtin types: `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`, `f32`, `f64`, `bool`, `unit`

### Tag Names (for variants)

Tag names start with an uppercase letter: `[A-Z][A-Za-z0-9_]*`
Examples: `UP`, `DOWN`, `LEFT`, `RIGHT`

### Layout (optional, for structs)

- `OPTIMAL` (default) -- compiler chooses layout
- `PACKED` -- no padding
- `C_ABI` -- C-compatible layout

## R-Node (Region Definitions)

```
R:<id> = region { lifetime: static }
R:<id> = region { lifetime: scoped, parent: R:<parent_id> }
```

Regions define memory lifetimes. Scoped regions have a parent region.

## C-Node (Compute Definitions)

All C-nodes are pure (no side effects).

### const
```
C:<id> = const { value: <literal>, type: <type_ref> }
C:<id> = const { value: <literal>, type: <type_ref>, region: R:<id> }
```

Literal values: integer (`-?[0-9]+`), float (`-?[0-9]+.[0-9]+`), boolean (`true`/`false`), string (`"..."`).

### const_bytes
```
C:<id> = const_bytes { value: [<byte>, ...], type: <type_ref>, region: R:<id> }
```

Byte array literal. Each byte is an integer 0-255.

### Arithmetic / Logic Operations
```
C:<id> = <op> { inputs: [<node_ref>, ...], type: <type_ref> }
```

Operators: `add`, `sub`, `mul`, `div`, `mod`, `and`, `or`, `xor`, `shl`, `shr`, `neg`, `not`, `eq`, `neq`, `lt`, `lte`, `gt`, `gte`

### call_pure
```
C:<id> = call_pure { target: "<function_name>", inputs: [<node_ref>, ...], type: <type_ref> }
```

Calls a pure named function. Target is a string literal.

### Atomic Operations (Concurrency)

```
C:<id> = atomic_load { source: M:<id>, order: <memory_order>, type: <type_ref> }
C:<id> = atomic_store { target: M:<id>, value: <node_ref>, order: <memory_order> }
C:<id> = atomic_cas { target: M:<id>, expected: <node_ref>, desired: <node_ref>, order: <memory_order>, success: <node_ref>, failure: <node_ref> }
```

Memory orders: `SEQ_CST`, `ACQUIRE_RELEASE`, `ACQUIRE`, `RELEASE`, `RELAXED`

### Generic Compute Operation (catch-all)
```
C:<id> = <op_name> { <key>: <value>, ... }
```

For operations not covered above (e.g., `bhaskara_approx`, `load`). Field values can be: node_ref, node_ref_list, type_ref, literal, effect_list, byte_array, memory_order, tag_name, ident.

## E-Node (Effect Definitions)

E-nodes represent side effects.

### Syscalls

```
E:<id> = syscall_exit { inputs: [<node_ref>, ...], type: <type_ref>, effects: [<effect>, ...] }
```

`syscall_exit` has no success/failure continuations.

```
E:<id> = syscall_write { inputs: [<fd>, <buf>, <len>], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
E:<id> = syscall_read { inputs: [<fd>, <buf>, <len>], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
E:<id> = syscall_open { inputs: [<path>], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
E:<id> = syscall_close { inputs: [<fd>], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
E:<id> = syscall_ioctl { inputs: [<fd>, <request>, <arg>], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
```

### call_extern (FFI)
```
E:<id> = call_extern { target: X:<id>, inputs: [<node_ref>, ...], type: <type_ref>, effects: [<effect>, ...], success: <node_ref>, failure: <node_ref> }
```

### Generic Effect (catch-all)
```
E:<id> = <op_name> { <key>: <value>, ... }
```

### Effect Names

Effect names are uppercase identifiers: `[A-Z][A-Za-z0-9_]*`
Common effects: `IO`, `MEM`, `PROC`

## K-Node (Control Flow)

### seq
```
K:<id> = seq { steps: [<node_ref>, ...] }
```

Sequential execution of steps.

### branch
```
K:<id> = branch { condition: <node_ref>, true: <node_ref>, false: <node_ref> }
```

Conditional branch. `condition` must evaluate to boolean.

### loop
```
K:<id> = loop { condition: <node_ref>, body: <node_ref>, state: <node_ref>, state_type: <type_ref> }
```

Loop with mutable state. Runs `body` while `condition` is true.

### par (Parallel)
```
K:<id> = par { branches: [<node_ref>, ...], sync: <BARRIER|NONE> }
K:<id> = par { branches: [<node_ref>, ...], sync: <BARRIER|NONE>, memory_order: <memory_order> }
```

Parallel execution of branches with synchronization mode.

## V-Node (Contract Definitions)

```
V:<id> = contract { target: <node_ref>, <clause>, ... }
```

A contract binds to a target node and contains one or more clauses.

### Clause Types

```
pre: <formula>
post: <formula>
invariant: <formula>
assume: <formula>
trust: <PROVEN|EXTERN>
```

Multiple clauses can appear in a single contract, comma-separated:
```
V:<id> = contract { target: E:d1, trust: EXTERN, assume: result != 0, post: result != 0 }
```

## X-Node (Extern / FFI Declarations)

```
X:<id> = extern { name: "<symbol>", abi: <C|SYSTEM_V|AAPCS64>, params: [<type_ref>, ...], result: <type_ref> }
X:<id> = extern { name: "<symbol>", abi: <C|SYSTEM_V|AAPCS64>, params: [<type_ref>, ...], result: <type_ref>, effects: [<effect>, ...] }
```

## M-Node (Memory Operations)

### alloc
```
M:<id> = alloc { type: <type_ref>, region: R:<id> }
```

### load
```
M:<id> = load { source: M:<id>, index: <node_ref>, type: <type_ref> }
```

### store
```
M:<id> = store { target: M:<id>, index: <node_ref>, value: <node_ref> }
```

## Entry Point

```
entry: <node_ref>
```

Exactly one entry point per program. Typically references a K-node.

## Formula Syntax (for V-Node contracts)

Operator precedence (low to high): `OR` < `AND` < `NOT` < comparison < `+`/`-` < `*`/`/`/`%` < unary

### Logical Operators
```
<formula> AND <formula>
<formula> OR <formula>
NOT <formula>
```

Note: use `AND`/`OR`/`NOT` (uppercase), not `&&`/`||`/`!`.

### Comparison Operators
```
<expr> == <expr>
<expr> != <expr>
<expr> < <expr>
<expr> > <expr>
<expr> <= <expr>
<expr> >= <expr>
```

### Arithmetic Operators (within formulas)
```
<expr> + <expr>
<expr> - <expr>
<expr> * <expr>
<expr> / <expr>
<expr> % <expr>
-<expr>
```

### Quantifiers
```
forall <var> in <start>..<end>: <formula>
forall (<var1>, <var2>) in <collection>: <formula>
```

### Special Tokens
```
result    // return value of target node
state     // loop state variable
null      // null pointer
```

### Field Access
```
<node_ref>.<field>
<node_ref>.<field>.<subfield>
state.<field>
state.<field>[<index>].<subfield>
```

### Index Access
```
<base>[<index>]
<base>[<index>].<field>
```

### Function Calls in Formulas
```
region_valid(R:<id>)
shared_mnodes(<ident>, <ident>)
all_accesses_atomic(<ident>, <ident>)
```

### Empty Set
```
{}
```

### Boolean Literals
```
true
false
```

### Parentheses
```
(<formula>)
```

## Struct/Array/Variant Operations (C-Node)

These are parsed as generic compute ops or specific named ops:

```
C:<id> = array_length { ... }
C:<id> = struct_get { ... }
C:<id> = struct_set { ... }
C:<id> = variant_tag { ... }
C:<id> = variant_get { ... }
C:<id> = variant_wrap { ... }
```

## Complete Example: Hello World

```ftl
// Types
T:a1 = array { element: u8, max_length: 12 }
T:a2 = integer { bits: 64, signed: false }
T:a3 = unit

// Regions
R:b1 = region { lifetime: static }

// Compute-Nodes
C:c1 = const_bytes { value: [72,101,108,108,111,32,87,111,114,108,100,10], type: T:a1, region: R:b1 }
C:c2 = const { value: 1, type: T:a2 }
C:c3 = const { value: 12, type: T:a2 }
C:c4 = const { value: 0, type: T:a2 }
C:c5 = const { value: 1, type: T:a2 }

// Effect-Nodes
E:d1 = syscall_write { inputs: [C:c2, C:c1, C:c3], type: T:a2, effects: [IO], success: K:f2, failure: K:f3 }

// Success path: exit(0)
K:f2 = seq { steps: [E:d2] }
E:d2 = syscall_exit { inputs: [C:c4], type: T:a3, effects: [PROC] }

// Failure path: exit(1)
K:f3 = seq { steps: [E:d3] }
E:d3 = syscall_exit { inputs: [C:c5], type: T:a3, effects: [PROC] }

// Contracts
V:e1 = contract { target: E:d1, pre: C:c2.val == 1 }
V:e2 = contract { target: E:d1, pre: C:c3.val == 12 }

// Entry
K:f1 = seq { steps: [E:d1] }
entry: K:f1
```

## Validation Pipeline

1. Parse (PEG grammar)
2. Structural validation (dangling refs, duplicate IDs)
3. Type and effect checking
4. Region checking (lifetime safety)
5. Contract proving (Z3/BMC)
6. Compilation to content-addressed binary graph

## Error Codes

- 1000-1999: Structural validation errors (fatal)
- 2000-2999: Warnings (non-fatal)
- 3000+: Type/effect/region errors (fatal)
