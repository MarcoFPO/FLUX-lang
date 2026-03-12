# Simulation: Hello World durch die FLUX v3 Pipeline

## Anforderung

```
User → LLM: "Gib den Text 'Hello World' auf der Konsole aus"
```

## Session-Start

```json
{ "action": "START",
  "type": "TRANSLATE",
  "target": "ANY",
  "constraints": {} }
```


## Pipeline-Durchlauf

### Iteration 1: LLM erzeugt FTL

```
T:a1 = array { element: u8, max_length: 12 }
T:a2 = integer { bits: 64, signed: false }
T:a3 = unit

R:b1 = region { lifetime: static }

C:c1 = const_bytes { value: [72,101,108,108,111,32,87,111,114,108,100,10],
                     type: T:a1, region: R:b1 }
C:c2 = const { value: 1, type: T:a2 }
C:c3 = const { value: 12, type: T:a2 }
C:c4 = const { value: 0, type: T:a2 }
C:c5 = const { value: 1, type: T:a2 }

E:d1 = syscall_write { inputs: [C:c2, C:c1, C:c3], type: T:a2,
                       effects: [IO], success: K:f2, failure: K:f3 }

K:f2 = seq { steps: [E:d2] }
E:d2 = syscall_exit { inputs: [C:c4], type: T:a3, effects: [PROC] }

K:f3 = seq { steps: [E:d3] }
E:d3 = syscall_exit { inputs: [C:c5], type: T:a3, effects: [PROC] }

V:e1 = contract { target: E:d1, pre: C:c2.val == 1 }
V:e2 = contract { target: E:d1, pre: C:c3.val == 12 }

K:f1 = seq { steps: [E:d1] }
entry: K:f1
```

- FTL-Compiler: **PASS** (syntaktisch korrekt)
- Validator: **PASS** (7/7 Checks — Typen, Effekte, Regionen, E-Node-Zweige)

### Contract Prover (Phase 1: Z3, < 1s)

```
V:e1: C:c2.val == 1    → PROVEN (trivial, 0.1ms)
V:e2: C:c3.val == 12   → PROVEN (trivial, 0.1ms)
```

Beide Contracts in Phase 1 geloest. Keine Eskalation noetig.

### System-Feedback

```json
{
  "status": "COMPILED",
  "graph_hash": "blake3:7f2a3b...",
  "contract_results": [
    { "contract_id": "V:e1", "status": "PROVEN", "prover_phase": 1 },
    { "contract_id": "V:e2", "status": "PROVEN", "prover_phase": 1 }
  ],
  "compilation_result": {
    "binary_hash": "blake3:e8c1d9...",
    "binary_size": 137,
    "targets": {
      "x86_64":  { "instructions": 12, "cycles_estimate": 8,
                   "superopt_improvement_pct": 0.0 },
      "aarch64": { "instructions": 13, "cycles_estimate": 9,
                   "superopt_improvement_pct": 0.0 },
      "riscv64": { "instructions": 14, "cycles_estimate": 10,
                   "superopt_improvement_pct": 0.0 }
    }
  }
}
```

Superoptimierung: Nur Stufe 1 (LLVM -O3) angewendet. Bei 12 Instruktionen
liegt der Code bereits am Optimum — Stufe 2 (MLIR) und Stufe 3 (STOKE) bringen
keine Verbesserung.


## Erzeugter Maschinencode

**x86-64 (137 Bytes, kein libc):**
```asm
_start:
    mov rax, 1          ; sys_write
    mov rdi, 1          ; stdout
    lea rsi, [rip+msg]  ; buffer
    mov rdx, 12         ; length
    syscall
    test rax, rax       ; E:d1 success/failure
    js .failure         ;   → failure-Pfad
    mov rax, 60         ; sys_exit(0) — success-Pfad
    xor edi, edi
    syscall
.failure:
    mov rax, 60         ; sys_exit(1) — failure-Pfad
    mov edi, 1
    syscall
msg: .ascii "Hello World\n"
```

**AArch64 (160 Bytes):**
```asm
_start:
    mov x8, #64         ; sys_write
    mov x0, #1          ; stdout
    adr x1, msg
    mov x2, #12
    svc #0
    tbnz x0, #63, .failure
    mov x8, #93         ; sys_exit(0)
    mov x0, #0
    svc #0
.failure:
    mov x8, #93         ; sys_exit(1)
    mov x0, #1
    svc #0
```

**RISC-V 64 (180 Bytes):**
```asm
_start:
    li a7, 64           ; sys_write
    li a0, 1            ; stdout
    la a1, msg
    li a2, 12
    ecall
    bltz a0, .failure
    li a7, 93           ; sys_exit(0)
    li a0, 0
    ecall
.failure:
    li a7, 93           ; sys_exit(1)
    li a0, 1
    ecall
```


## Ergebnis

```
Gesamtzeit:          ~1.2s (davon 1.0s = LLM-Latenz)
KI-Iterationen:      1 (kein Fehler, kein Feedback noetig)
Contracts bewiesen:  2/2 (Phase 1: Z3, je 0.1ms)
Runtime-Checks:      0
Superoptimierung:    Stufe 1 (kein Verbesserungspotential)
Binaries:            identisch zu handgeschriebenem Assembler
```


## Vergleich v2 → v3 (an diesem Beispiel)

```
Aspekt                v2                           v3
──────────────────────────────────────────────────────────────
LLM-Input             JSON (binaer)                FTL (Text)
Fehlerbehandlung      F-Node (fault_policy)        E-Node success/failure
Validator-Feedback    Unspezifiziert               JSON mit error_code + suggestion
Contract-Sprache      Informell                    SMT-LIB2-kompatibel (QF_LIA)
Prover-Strategie      Einzelner Durchlauf          Gestaffelt (hier: Phase 1 genuegt)
System-Output         Unspezifiziert               JSON mit Compilation-Metriken
Superoptimierung      LLVM -O3                     3-stufig (hier: nur Stufe 1)
```
