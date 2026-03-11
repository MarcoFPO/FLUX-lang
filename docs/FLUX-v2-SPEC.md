# FLUX v2 — Vollstaendige technische Spezifikation

## 1. Ueberarbeitete Architektur

Die zentrale Erkenntnis: Codegeneratoren fuer ARM/x86/RISC-V existieren bereits (LLVM). FLUX baut darauf auf, statt sie neu zu erfinden.

```
┌──────────────────────────────────────────────────────────────┐
│  ANFORDERUNG                                                  │
│  Natuerliche Sprache oder formale Spezifikation               │
└───────────────────────┬──────────────────────────────────────┘
                        │
┌───────────────────────▼──────────────────────────────────────┐
│  KI-GENERATOR                                                 │
│  LLM erzeugt strukturiertes JSON → Deserializer → Graph      │
│  Iterative Korrekturschleife bei Validierungsfehlern         │
└───────────────────────┬──────────────────────────────────────┘
                        │  FLUX Computation Graph (binaer)
┌───────────────────────▼──────────────────────────────────────┐
│  VERIFIKATION                                                 │
│  Strukturpruefung → Typpruefung → SMT (begrenzte Fragmente)  │
│  Timeout: 5s pro Contract, danach → Runtime-Check            │
└───────────────────────┬──────────────────────────────────────┘
                        │  Verifizierter Graph
┌───────────────────────▼──────────────────────────────────────┐
│  FLUX→MLIR LOWERING                                          │
│  FLUX-Dialekt → Standard-MLIR-Dialekte → LLVM-Dialekt       │
│  Graph-Optimierungen auf MLIR-Ebene                          │
└───────────────────────┬──────────────────────────────────────┘
                        │  LLVM IR
┌───────────────────────▼──────────────────────────────────────┐
│  LLVM BACKEND (existierend, nicht neu gebaut)                │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐        │
│  │ AArch64 │  │ x86-64  │  │ RISC-V  │  │  WASM   │        │
│  │ NEON    │  │ AVX-512 │  │ RVV     │  │ SIMD128 │        │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘        │
│   .so/.dylib    .so/.exe     .elf          .wasm            │
└──────────────────────────────────────────────────────────────┘
```

## 2. Node-Typen

```
C-Node  (Compute)     Reine Berechnung
E-Node  (Effect)      Seiteneffekte (IO, Syscall, Netzwerk)
K-Node  (Kompose)     Zusammensetzung: Seq | Par | Branch | Loop | Finally
V-Node  (Verify)      Contract (Pre/Post/Invariant)
T-Node  (Type)        Typ-Definition mit Constraints
H-Node  (Hint)        Hardware-Hint (SIMD, Cache, Alignment)
M-Node  (Memory)      Speicheroperation mit Region-Tag
F-Node  (Fault)       Fehlerbehandlung und Error-Propagation
R-Node  (Region)      Speicherregion (Lifetime-Scope)
D-Node  (Debug)       Debug-Mapping (Node → Anforderung)
P-Node  (Package)     Modul-Grenze mit typisiertem Interface
```

## 3. Binaeres Format

```
┌─────────────────────────────────────────────┐
│ FLUX Binary Format v2                        │
├─────────────────────────────────────────────┤
│ Header:                                      │
│   magic:        0x464C5558 ("FLUX")          │
│   version:      u16                          │
│   target_hint:  enum { Any, ARM64, X86_64,   │
│                        RISCV64, WASM }       │
│   node_count:   u32                          │
│   type_count:   u16                          │
│   region_count: u16                          │
├─────────────────────────────────────────────┤
│ Type Table:                                  │
│   [T-Nodes: id, kind, size, align, constr]   │
├─────────────────────────────────────────────┤
│ Region Table:                                │
│   [R-Nodes: id, parent, lifetime, max_size]  │
├─────────────────────────────────────────────┤
│ Node Table:                                  │
│   [Nodes: id, kind, type_in[], type_out,     │
│    op, inputs[], region, faults[], hints[]]  │
├─────────────────────────────────────────────┤
│ Edge Table:                                  │
│   [from_node, to_node, edge_kind]            │
│   edge_kind: Data | Control | Effect | Fault │
├─────────────────────────────────────────────┤
│ Contract Table:                              │
│   [V-Nodes: target_node, kind, formula,      │
│    proven: bool, check_strategy]             │
├─────────────────────────────────────────────┤
│ Debug Table:                                 │
│   [D-Nodes: node_id, requirement_id,         │
│    description, intent_hash]                 │
├─────────────────────────────────────────────┤
│ Content Hash: BLAKE3(alle obigen Sections)   │
└─────────────────────────────────────────────┘
```

## 4. Speichermodell — Region-basiert

Kein Garbage Collector. Kein manuelles malloc/free. Region-basierte Arena-Allokation.

### Regeln (vom Compiler erzwungen):

1. Jedes M-Node (ALLOC) gehoert zu genau einer R-Node (Region)
2. Referenzen duerfen nur in die eigene oder eine aeussere Region zeigen
3. Am Ende eines K-Node-Scopes werden alle inneren Regionen deterministisch freigegeben
4. Kein FREE noetig — Regionen werden als Block freigegeben

### Speicher-Ordnung bei Concurrency:

5. Zwischen SPAWN/JOIN: Sequentially Consistent
6. Innerhalb eines Threads: Program Order
7. ATOMIC-Ops: Acquire/Release-Semantik (explizit im Node)
8. BARRIER: Full Fence auf Hardware-Ebene

### Lowering zu LLVM:

```
R-Node (Region ALLOC)  →  llvm.lifetime.start + arena allocator
R-Node (Region FREE)   →  llvm.lifetime.end + bulk free
M-Node (LOAD)          →  llvm load mit tbaa metadata
M-Node (STORE)         →  llvm store mit tbaa metadata
ATOMIC                 →  llvm atomicrmw / cmpxchg
BARRIER                →  llvm fence
```

## 5. Fehlerbehandlung

Zwei Fehlerklassen:

**RECOVERABLE (F-Node mit Recovery-Pfad):**
Netzwerk-Timeout, ungueltige Eingabe, Datei nicht gefunden.
Werden als alternative Kanten im Graph modelliert.
KI MUSS einen Recovery-Pfad erzeugen (erzwungen durch Typsystem).

**FATAL (Hardware-Trap):**
Division durch Null (unbewiesen), Stack Overflow, OOM.
Werden zu Hardware-Traps gesenkt.
Deterministischer Prozess-Abort mit Debug-Info.

Kompiliert zu einfachen Branch-Instruktionen. Kein Exception-Stack-Unwinding. Null Runtime-Overhead im Erfolgsfall.

## 6. Modulsystem (P-Nodes)

```
P-Node #http_server {
  interface: {
    exports: [fn handle_request(Request) -> Response faults: [Timeout]]
    imports: [fn db_query(SQL) -> Rows effects: [DB]]
  }
  content_hash: BLAKE3(alle enthaltenen Nodes)
}
```

Eigenschaften:
- Typisiertes Interface: Exporte und Importe vollstaendig deklariert
- Content-Addressiert: Gleicher interner Graph = gleicher Hash
- Keine zirkulaeren Abhaengigkeiten (DAG auch auf Modul-Ebene)
- Effekt-Transparenz: Import-Effekte propagieren zum P-Node-Interface

## 7. Operationskatalog

### ARITHMETIC
ADD, SUB, MUL, DIV, MOD — mit Overflow-Policy: checked | wrapping | saturating

### LOGIC
AND, OR, XOR, NOT, SHIFT_L, SHIFT_R

### MEMORY
LOAD(address, type, alignment, cache_policy)
STORE(address, value, alignment, cache_policy)
ALLOC(size, alignment, region)
— Kein FREE (Regionen werden als Block freigegeben)

### CONTROL
BRANCH(condition, true_node, false_node)
LOOP(condition, body, invariant)
CALL(node_ref, args)
RETURN(value)
FINALLY(body, cleanup) — Garantiertes Cleanup

### VECTOR
VLOAD(address, width) — 128/256/512-bit, hardware-agnostisch
VCOMPUTE(op, vector_a, vector_b) — Backend waehlt NEON/AVX/RVV
VREDUCE(op, vector)

### CONCURRENT
SPAWN(node, args)
JOIN(handles)
ATOMIC(op, address, value) — Acquire/Release
BARRIER(scope) — local | global

## 8. Runtime (~50 KB)

```
flux_rt.a / flux_rt.so
├── Region Allocator (arena_create, arena_alloc, arena_destroy)
├── Concurrency Scheduler (task_spawn, task_join, atomic_ops)
├── Platform Syscall Bridge (Linux direct / macOS libSystem / Windows ntdll)
├── Fault Handler (SIGSEGV/SIGFPE → Debug-Dump via D-Node-Mapping)
└── Debug Support (optional, entfernbar) — Execution Trace Recorder
```

## 9. Maschinencode-Erzeugung

### FLUX → MLIR Lowering

```
flux.compute   → arith/math Dialekt
flux.effect    → llvm.call mit Side-Effect-Markierung
flux.region    → memref.alloc mit Lifetime-Scope
flux.branch    → scf.if / scf.for
flux.parallel  → async.launch / omp.parallel
flux.vector    → vector Dialekt
flux.fault     → scf.if (Fehler-Branch)
flux.atomic    → llvm.atomicrmw
```

### MLIR Optimierungs-Passes

```
1. flux-canonicalize      Normalisierung
2. flux-verify-contracts  SMT-Pruefung, bewiesene V-Nodes entfernen
3. flux-inline-small      Kleine P-Nodes inlinen
4. flux-region-merge      Benachbarte Regionen zusammenfuehren
5. flux-to-std            FLUX-Ops → Standard-MLIR
6. convert-to-llvm        → LLVM-Dialekt
7. llvm-legalize          LLVM-IR normalisieren
```

### Ziel-Plattformen

```
Plattform    ISA        SIMD           System-ABI      Ausgabe
Linux ARM64  AArch64    NEON/SVE2      AAPCS64         ELF .so
Linux x86    AMD64      SSE4.2/AVX2    System V AMD64  ELF .so
Linux RISC-V RV64GC     RVV 1.0        RISC-V LP64D    ELF .so
macOS ARM64  AArch64    NEON            Apple AAPCS64   Mach-O .dylib
Windows x86  AMD64      AVX2            Microsoft x64   PE .dll
WASM         WASM32/64  SIMD128         WASI-Preview2   .wasm
Bare Metal   variabel   variabel        custom          flat binary
```

## 10. KI-Generierungspipeline

```
LLM (Structured Output / JSON)
    ↓
FLUX VALIDATOR (deterministisch)
  1. Schema-Validierung
  2. Struktur-Check (DAG, keine Zyklen)
  3. Typ-Check (type_in/type_out konsistent)
  4. Effekt-Check (E-Nodes deklariert)
  5. Fault-Check (Recovery-Pfade vorhanden)
  6. Region-Check (keine Escapes)
    ↓ Bei Fehler: strukturiertes Feedback → zurueck ans LLM (max 3 Iterationen)
    ↓
CONTRACT PROVER (Z3/CVC5, Timeout 5s/Contract)
  PROVEN    → V-Node entfernen, 0 Overhead
  DISPROVEN → Fehler zurueck ans LLM
  TIMEOUT   → Runtime-Check einfuegen (Branch)
    ↓
MLIR → LLVM → Maschinencode
```

## 11. Debugging

```
flux-trace replay recording.ftrace    Ausfuehrung Schritt fuer Schritt
flux-trace blame #07                  Welche Anforderung? Welche KI-Session?
flux-graph render app.flux -o app.svg Graph visualisieren
flux-graph diff v1.flux v2.flux       Struktureller Diff
```

## 12. Phasenplan

```
Phase 1 — Fundament (6 Monate)
├── FLUX Binary Format Spezifikation (formal)
├── FLUX Validator (Rust)
├── FLUX MLIR Dialekt (C++)
└── Erstes Ziel: Arithmetik → x86-64 Binary

Phase 2 — Speicher + Fehler (6 Monate)
├── Region Allocator Runtime
├── M-Node / R-Node / F-Node Lowering
└── Concurrency: SPAWN/JOIN

Phase 3 — KI-Integration (6 Monate)
├── LLM System-Prompt + Schema
├── Validator-Feedback-Loop
├── Synthetischer Trainingskorpus
└── SMT-Integration

Phase 4 — Multi-Target + Oekosystem (12 Monate)
├── ARM64 + RISC-V + WASM Backends
├── P-Node Linking + Modul-Registry
├── Debug-Tooling
└── Benchmark-Suite
```
