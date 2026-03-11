# Simulation: Snake Game durch die FLUX v2 Pipeline

## Anforderung

```
User → KI: "Erstelle ein Snake-Game mit Soundausgabe fuer das Linux-Terminal"
```

## Komplexitaet

- 6 Module (Terminal, Sound, Game Logic, Renderer, Timing, Main)
- 118+ Nodes, 187 Kanten
- 18 Typen (inkl. game_state, snake_body, pcm_buffer)
- 4 Regionen (static, game, frame, sound)
- 7 Contracts

## Pipeline-Durchlauf

### Iteration 1: KI erzeugt Graph
- 118 Nodes, 6 P-Nodes
- Validator: **2 FAIL**
  1. Region-Escape: `n_write_screen` liest Framebuffer aus `r_frame` ohne Region-Annotation
  2. Fehlender Cleanup: Terminal bleibt im Raw-Modus bei Abort (Shell kaputt!)
- Warning: SIN() in Sine-Generator verhindert Vektorisierung

### Iteration 2: KI korrigiert
- Region-Annotation hinzugefuegt
- `main` mit K-Node(FINALLY) gewrappt fuer garantiertes `term_restore`
- SIN() durch Bhaskara-I-Polynom ersetzt (vektorisierbar)
- Validator: **PASS**, aber Contract Prover findet Buffer-Overflow

### Contract Prover — DISPROVEN
```
v_framebuf_size: Framebuffer max 8192 Bytes
Worst-Case: 40x20 Grid x ~12 Bytes/Zelle + ANSI-Header = ~9800 Bytes
→ BUFFER ZU KLEIN!
```

### Iteration 3: KI erhoeht Buffer
- `t_framebuf.max_length`: 8192 → 16384
- Validator: **PASS**
- Prover: 6/7 PROVEN, 1 TIMEOUT (v_snake_in_bounds → Runtime-Check)

## Erzeugter Maschinencode (x86-64)

### Binary-Statistik
```
.text:          4,892 Bytes  (Maschinencode)
.rodata:          648 Bytes  (Konstanten, ANSI-Strings)
.data:             16 Bytes  (RNG-State)
flux_rt:        3,104 Bytes  (Arena, Scheduler, Syscall)
ELF Headers:      872 Bytes
────────────────────────────
TOTAL:          9,532 Bytes  (~9.3 KB, statisch, kein libc)
```

### Vergleich
```
FLUX Snake:     9.3 KB   (statisch, keine Deps)
C (gcc -Os):   18.2 KB   (statisch, musl)
Rust:          42.0 KB   (statisch)
Go:             1.9 MB   (statisch)
Python:         3.2 KB   (+ Python Runtime ~30 MB)
```

## Laufzeit-Trace (Auszug)

```
[T+0.000ms] main ENTER
[T+0.012ms] term_init → Raw-Modus, Echo aus, Cursor versteckt
[T+0.015ms] sound_init → ALSA PCM fd=4, 44100 Hz, S16_LE
[T+0.018ms] game_init → Snake [{20,10}], Food {7,14}, Score 0

--- TICK 1 ---
[T+0.020ms] read_key → KEY_NONE (nichts gedrueckt)
[T+0.021ms] update_game → Snake bewegt: {20,10} → {21,10}
[T+0.022ms] render_game → 2847 Bytes ANSI (pure, kein IO)
[T+0.024ms] write_frame → Terminal aktualisiert
[T+0.170ms] sleep → naechster Tick

--- TICK mit FUTTER ---
[T+3.771ms] update_game → HEAD == FOOD! Score: 0 → 10
[T+3.771ms] play_tone → 880 Hz, 50ms (AVX2: 276 Iterationen, 0.003ms)
[T+3.771ms] 🔊 Fress-Sound!

--- GAME OVER (Tick 847) ---
[T+127.0s] update_game → Selbstkollision bei {14,8}
[T+127.0s] play_tone → 220 Hz, 500ms Tod-Sound
[T+127.5s] FINALLY → sound_close + term_restore (garantiert!)
[T+127.5s] exit(0)
```

## Zusammenfassung

```
Anforderung:        6 Woerter
KI-Iterationen:     3
Fehler gefunden:    3 (Region-Escape, fehlender Cleanup, Buffer-Overflow)
Contracts bewiesen: 6/7 (compile-time, 0 Overhead)
Runtime-Checks:     1 (bounds check, ~2ns/Tick)
Binary:             9.3 KB, 0 externe Deps
Speicher-Peak:      ~20 KB (arena-basiert, 0 GC-Pausen)
CPU-Last:           ~0.1%
```
