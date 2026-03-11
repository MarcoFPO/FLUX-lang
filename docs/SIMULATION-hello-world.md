# Simulation: Hello World durch die FLUX v2 Pipeline

## Anforderung

```
User → KI: "Gib den Text 'Hello World' auf der Konsole aus"
```

## Pipeline-Durchlauf

### Iteration 1: KI erzeugt Graph
- 7 Nodes, 4 Typen, 2 Regionen
- Validator: **FAIL** — E-Node `n_write` hat keinen F-Node (Syscall kann fehlschlagen)

### Iteration 2: KI korrigiert
- +1 F-Node mit `fault_policy: abort`
- Validator: **PASS** (6/6 Checks)

### Contract Prover
```
v_fd_valid:      n_fd.value == 1         → PROVEN (0.1ms, entfernt)
v_len_matches:   n_len.value == 12       → PROVEN (0.1ms, entfernt)
```
Ergebnis: 2/2 Contracts bewiesen, 0 Runtime-Checks

### Erzeugter Maschinencode

**x86-64 (137 Bytes, kein libc):**
```asm
_start:
    mov rax, 1          ; sys_write
    mov rdi, 1          ; stdout
    lea rsi, [rip+msg]  ; buffer
    mov rdx, 12         ; length
    syscall
    test rax, rax
    js .fault
    mov rax, 60         ; sys_exit
    xor edi, edi        ; code 0
    syscall
.fault:
    mov rax, 60
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
    tbnz x0, #63, .fault
    mov x8, #93         ; sys_exit
    mov x0, #0
    svc #0
.fault:
    mov x8, #93
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
    bltz a0, .fault
    li a7, 93           ; sys_exit
    li a0, 0
    ecall
.fault:
    li a7, 93
    li a0, 1
    ecall
```

### Ergebnis

```
Gesamtzeit: ~2.1s (davon 2.0s = LLM-Latenz)
KI-Iterationen: 2
Fehler gefunden + korrigiert: 1 (fehlender F-Node)
Binaries: identisch zu handgeschriebenem Assembler
```
