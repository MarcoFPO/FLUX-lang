# Simulation: Snake Game durch die FLUX v3 Pipeline

## Anforderung

```
User → LLM: "Erstelle ein Snake-Game mit Soundausgabe fuer das Linux-Terminal"
```

## Session-Start

```json
{ "action": "START",
  "type": "TRANSLATE",
  "target": "X86_64",
  "constraints": { "max_binary_size": 65536, "max_memory": 1048576 } }
```


## Komplexitaet

```
Funktionsbereiche:  6 (Terminal, Sound, Game Logic, Renderer, Timing, Main)
Nodes:              118+ (flacher Graph, keine P-Nodes)
Kanten:             187
Typen:              18 (inkl. game_state, snake_body, pcm_buffer)
Regionen:           4 (static, game, frame, sound)
Contracts:          7
```

Anmerkung: v3 hat keine P-Nodes (Module). Der Graph ist flach.
Funktionsbereiche sind eine logische Gruppierung, keine Sprachkonstrukte.


## Pipeline-Durchlauf

### Iteration 1: LLM erzeugt FTL

LLM liefert 118 Nodes als FTL-Text:

```
// Typen (Auszug)
T:a1 = struct { fields: [x: T:a10, y: T:a10] }
T:a2 = array { element: T:a1, max_length: 800 }
T:a3 = struct { fields: [snake: T:a2, length: T:a10, dir: T:a11,
                          food: T:a1, score: T:a10, alive: T:a12] }
T:a4 = array { element: u8, max_length: 8192 }
T:a5 = array { element: i16, max_length: 2048 }
T:a10 = integer { bits: 32, signed: true }
T:a11 = variant { cases: [UP: unit, DOWN: unit, LEFT: unit, RIGHT: unit] }
T:a12 = boolean

// Regionen
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
R:b3 = region { lifetime: scoped, parent: R:b2 }
R:b4 = region { lifetime: scoped, parent: R:b1 }

// Terminal-Init (E-Node mit success/failure)
E:d1 = syscall_ioctl { inputs: [C:c1, C:c2, C:c3], type: T:a10,
                        effects: [IO], success: K:f10, failure: K:f_abort }

// Sound-Init
E:d2 = syscall_open { inputs: [C:c_alsa_path], type: T:a10,
                       effects: [IO], success: K:f11, failure: K:f_abort }

// Sine-Generator (pure Berechnung)
C:c_sin = bhaskara_approx { inputs: [C:c_angle], type: T:a13 }

// Game Loop
K:f1 = loop { condition: C:c_alive, body: K:f_tick,
              state: C:c_gamestate, state_type: T:a3 }

// Cleanup (garantiert via branch, nicht FINALLY)
K:f_main = branch { condition: C:c_true, true: K:f_game, false: K:f_abort }
// ... (118 Nodes gesamt)

// Contracts (SMT-LIB2-kompatibel)
V:e1 = contract { target: E:d1, pre: C:c1.val == 0 }
V:e2 = contract { target: K:f1, invariant:
                  forall i in 0..state.length: state.snake[i].x >= 0 }
V:e3 = contract { target: K:f1, invariant:
                  forall i in 0..state.length: state.snake[i].x < 40 }
V:e4 = contract { target: C:c_render, post: result.size <= T:a4.max_length }
V:e5 = contract { target: C:c_sin, post: result >= -1.0 AND result <= 1.0 }
V:e6 = contract { target: E:d2, pre: C:c_alsa_path != null }
V:e7 = contract { target: K:f1, invariant: state.length <= 800 }

entry: K:f_main
```

FTL-Compiler: **PASS** (syntaktisch korrekt)

Validator: **2 FAIL**

System-Feedback:
```json
{
  "status": "VALIDATION_FAIL",
  "graph_hash": "blake3:3a7c...",
  "validation_errors": [
    {
      "error_code": 2003,
      "node_id": "M:g5",
      "violation": "REGION_ESCAPE",
      "message": "M:g5 reads from R:b3 (frame) but E:d_write has no region annotation for R:b3",
      "context": {
        "source_region": "R:b3",
        "target_node": "E:d_write",
        "suggestion": "Add region annotation: E:d_write.regions += [R:b3]"
      }
    },
    {
      "error_code": 3001,
      "node_id": "K:f_main",
      "violation": "MISSING_FAILURE_PATH",
      "message": "K:f_main success-path terminates but failure-path E:d1.failure does not call term_restore",
      "context": {
        "effect": "IO (terminal raw mode)",
        "suggestion": "Ensure all E-Node failure paths that follow E:d1 include terminal restore sequence"
      }
    }
  ]
}
```


### Iteration 2: LLM korrigiert

LLM reagiert auf JSON-Feedback:

1. **Region-Escape:** Region-Annotation `R:b3` an `E:d_write` hinzugefuegt
2. **Terminal-Cleanup:** Alle failure-Pfade fuehren ueber `K:f_cleanup` das `term_restore` enthaelt.
   Kein F-Node, kein FINALLY — stattdessen konvergieren ALLE E-Node failure-Pfade
   auf denselben Cleanup-Subgraph:
   ```
   E:d1.failure ──→ K:f_cleanup ──→ E:d_restore ──→ E:d_exit
   E:d2.failure ──→ K:f_cleanup ──→ E:d_restore ──→ E:d_exit
   E:d_write.failure ──→ K:f_cleanup ──→ E:d_restore ──→ E:d_exit
   ```
3. **Optimierung:** SIN() durch Bhaskara-I-Approximation ersetzt (vektorisierbar)

```json
{ "action": "SUBMIT",
  "graphs": [{ "ftl": "...(korrigierter Graph)...",
               "parent_ref": "blake3:3a7c...",
               "generation": 1,
               "mutation_log": ["fix:region_escape:M:g5",
                                "fix:failure_path:K:f_main",
                                "opt:replace_sin:bhaskara"] }] }
```

FTL-Compiler: **PASS**
Validator: **PASS** (alle 14 Checks bestanden)


### Contract Prover — Gestaffelte Analyse

```
Phase 1 (Z3, Timeout 60s):
  V:e1: C:c1.val == 0 (fd=stdin)        → PROVEN (0.2ms, QF_LIA)
  V:e5: bhaskara result in [-1,1]        → PROVEN (12ms, QF_NIA)
  V:e6: ALSA-Pfad != null               → PROVEN (0.1ms, QF_LIA)
  V:e7: state.length <= 800             → PROVEN (3ms, QF_LIA)
  V:e4: render result <= 8192           → DISPROVEN!

Phase 1 Ergebnis: 4 PROVEN, 1 DISPROVEN, 2 ausstehend
```

System-Feedback fuer DISPROVEN:
```json
{
  "status": "CONTRACT_FAIL",
  "contract_results": [
    {
      "contract_id": "V:e4",
      "status": "DISPROVEN",
      "counterexample": {
        "bindings": {
          "grid_width": 40, "grid_height": 20,
          "bytes_per_cell": 12, "ansi_header": 200
        },
        "trace": ["C:c_render", "M:g5"],
        "explanation": "40 * 20 * 12 + 200 = 9800 > 8192. Framebuffer T:a4.max_length zu klein."
      }
    },
    { "contract_id": "V:e2", "status": "PENDING", "prover_phase": 1 },
    { "contract_id": "V:e3", "status": "PENDING", "prover_phase": 1 }
  ]
}
```


### Iteration 3: LLM erhoeht Buffer + Resubmit

LLM sieht Counterexample (9800 > 8192) und korrigiert:

```
// Vorher:
T:a4 = array { element: u8, max_length: 8192 }

// Nachher:
T:a4 = array { element: u8, max_length: 16384 }

// Contract angepasst:
V:e4 = contract { target: C:c_render, post: result.size <= 16384 }
```

```json
{ "action": "SUBMIT",
  "graphs": [{ "ftl": "...",
               "parent_ref": "blake3:8b2e...",
               "generation": 2,
               "mutation_log": ["fix:buffer_size:T:a4:8192->16384"] }] }
```

### Contract Prover — Vollstaendiger Durchlauf

```
Phase 1 (Z3, Timeout 60s):
  V:e1: fd == 0                          → PROVEN (0.2ms)
  V:e4: render <= 16384                  → PROVEN (8ms, 16384 > 9800)
  V:e5: bhaskara in [-1,1]              → PROVEN (12ms)
  V:e6: path != null                     → PROVEN (0.1ms)
  V:e7: length <= 800                   → PROVEN (3ms)

Phase 1 fuer V:e2, V:e3 (Array-Quantoren):
  V:e2: forall i: snake[i].x >= 0       → TIMEOUT nach 60s
  V:e3: forall i: snake[i].x < 40       → TIMEOUT nach 60s

Phase 2 (BMC, CBMC mit Bound k=100):
  V:e2: snake[i].x >= 0                 → PROVEN (47s, induktiv ueber Loop-Body)
  V:e3: snake[i].x < 40                 → PROVEN (52s, induktiv ueber Loop-Body)
```

Alle 7 Contracts bewiesen. Keine Inkubation noetig.

System-Feedback:
```json
{
  "status": "COMPILED",
  "graph_hash": "blake3:d4f1...",
  "contract_results": [
    { "contract_id": "V:e1", "status": "PROVEN", "prover_phase": 1 },
    { "contract_id": "V:e2", "status": "PROVEN", "prover_phase": 2 },
    { "contract_id": "V:e3", "status": "PROVEN", "prover_phase": 2 },
    { "contract_id": "V:e4", "status": "PROVEN", "prover_phase": 1 },
    { "contract_id": "V:e5", "status": "PROVEN", "prover_phase": 1 },
    { "contract_id": "V:e6", "status": "PROVEN", "prover_phase": 1 },
    { "contract_id": "V:e7", "status": "PROVEN", "prover_phase": 1 }
  ],
  "compilation_result": {
    "binary_hash": "blake3:a9e2...",
    "binary_size": 9532,
    "targets": {
      "x86_64": {
        "instructions": 847,
        "cycles_estimate": null,
        "superopt_improvement_pct": 8.3
      }
    }
  }
}
```


## Superoptimierung (3-stufig)

```
Stufe 1: LLVM -O3 (gesamter Graph)
  → Baseline-Optimierung, alle 847 Instruktionen

Stufe 2: MLIR-Level (Hot Paths, < 200 Ops)
  → render_game: Schleifenverschmelzung (Grid-Iteration + ANSI-Encoding)
  → update_game: Bounds-Check Hoisting aus Loop

Stufe 3: STOKE (Hot Paths, < 30 Instruktionen)
  → bhaskara_approx: 7 Instruktionen, AVX2-vektorisiert
  → pcm_fill: 11 Instruktionen, AVX2 256-bit
  → Verbesserung: 8.3% ueber reines LLVM -O3
```


## Erzeugter Maschinencode (x86-64)

### Binary-Statistik
```
.text:          4,892 Bytes  (Maschinencode)
.rodata:          648 Bytes  (Konstanten, ANSI-Strings)
.data:             16 Bytes  (RNG-State)
flux_rt:        3,104 Bytes  (Arena, Scheduler, Syscall-Wrapper)
ELF Headers:      872 Bytes
────────────────────────────
TOTAL:          9,532 Bytes  (~9.3 KB, statisch, kein libc)
```

### Vergleich
```
FLUX Snake:     9.3 KB   (statisch, keine Deps, 7/7 Contracts bewiesen)
C (gcc -Os):   18.2 KB   (statisch, musl, keine formale Verifikation)
Rust:          42.0 KB   (statisch, Borrow-Checker aber keine Contracts)
Go:             1.9 MB   (statisch, GC)
Python:         3.2 KB   (+ Python Runtime ~30 MB)
```


## Laufzeit-Trace (Auszug)

```
[T+0.000ms] main ENTER — Arena R:b1 (static) initialisiert
[T+0.012ms] E:d1 term_init → success-Pfad
             Raw-Modus, Echo aus, Cursor versteckt
[T+0.015ms] E:d2 sound_init → success-Pfad
             ALSA PCM fd=4, 44100 Hz, S16_LE
[T+0.018ms] C:c_init game_init
             Snake [{20,10}], Food {7,14}, Score 0
             Arena R:b2 (game) allokiert: 6.4 KB

--- TICK 1 ---
[T+0.020ms] E:d_read read_key → success (KEY_NONE)
[T+0.021ms] C:c_update update_game → Snake: {20,10} → {21,10}
[T+0.022ms] C:c_render render_game → 2847 Bytes ANSI in R:b3 (frame)
[T+0.024ms] E:d_write write_frame → success
[T+0.024ms] R:b3 (frame) freigegeben (Arena-Reset, 0 Fragmentation)
[T+0.170ms] sleep → naechster Tick

--- TICK mit FUTTER ---
[T+3.771ms] C:c_update → HEAD == FOOD! Score: 0 → 10
[T+3.771ms] C:c_sin bhaskara_approx(880 Hz)
             → AVX2: 276 Samples, 0.003ms (STOKE-optimiert)
[T+3.774ms] E:d_snd pcm_write → success (50ms Fress-Sound)

--- GAME OVER (Tick 847) ---
[T+127.0s]  C:c_update → Selbstkollision bei {14,8}
             state.alive = false → Loop-Condition false → Loop exit
[T+127.0s]  C:c_sin bhaskara_approx(220 Hz)
             → 500ms Tod-Sound
[T+127.5s]  K:f_cleanup → E:d_snd_close + E:d_restore (term_restore)
             Alle success-Pfade → sauberer Exit
[T+127.5s]  exit(0)
             R:b2 (game), R:b4 (sound) freigegeben (Arena-Destroy)
```


## Zusammenfassung

```
Anforderung:        6 Woerter
KI-Iterationen:     3
Fehler gefunden:    3 (Region-Escape, fehlende Failure-Pfade, Buffer-Overflow)
Feedback-Format:    JSON mit Counterexamples + Suggestions
Contracts bewiesen: 7/7 compile-time
  Phase 1 (Z3):    5 Contracts (< 1s)
  Phase 2 (BMC):   2 Contracts (< 60s)
Runtime-Checks:     0 (alle Contracts bewiesen!)
Superoptimierung:   3-stufig, 8.3% Verbesserung ueber LLVM -O3
Binary:             9.3 KB, 0 externe Deps
Speicher-Peak:      ~20 KB (arena-basiert, 0 GC-Pausen)
CPU-Last:           ~0.1%
```


## Vergleich v2 → v3 (an diesem Beispiel)

```
Aspekt                v2                           v3
──────────────────────────────────────────────────────────────
Graph-Struktur        6 P-Nodes (Module)           Flacher Graph (118 Nodes)
Fehlerbehandlung      F-Node (fault_policy)        E-Node success/failure Pfade
Terminal-Cleanup      K-Node(FINALLY)              Konvergierende failure-Pfade
Validator-Feedback    Text (unspezifiziert)         JSON mit error_code + suggestion
Contract V:e4         Informell formuliert          SMT-LIB2 (QF_LIA)
Buffer-Overflow       Counterexample (Text)         JSON mit konkreten Bindings
Snake-Bounds V:e2/3   TIMEOUT → Runtime-Check      Phase 2 (BMC) → PROVEN
Runtime-Checks        1 (bounds, ~2ns/Tick)         0 (alle bewiesen!)
Superoptimierung      LLVM -O3                      3-stufig (+8.3%)
```
