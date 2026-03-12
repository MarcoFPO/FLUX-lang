# FLUX v3 — Radikale Reduktion

## Design-Axiome

```
1. Compile-Zeit ist irrelevant           → Exhaustive Verifikation, Superoptimierung
2. Lesbarkeit ist irrelevant             → Kein Text, keine Namen, keine Kommentare
3. Menschliche Hilfskonstrukte entfallen → Kein Debug, kein Exception-Handling,
                                           keine defensive Programmierung,
                                           keine Abstraktion zur Verstaendlichkeit
4. Performance der Codegenerierung       → Beliebig viele LLM-Iterationen,
   ist sekundaer                           beliebig tiefe Analyse
```

## Was gegenueber v2 wegfaellt

```
ENTFERNT          BEGRUENDUNG
──────────────────────────────────────────────────────────────────
D-Node (Debug)    Kein Mensch liest den Code. Tracing unnoetig.
H-Node (Hint)     Compile-Zeit unbegrenzt → Superoptimizer findet
                  optimale Instruktionen automatisch.
P-Node (Package)  Module existierten zur menschlichen Organisation.
                  KI arbeitet mit einem flachen Gesamt-Graph.
FINALLY           Menschliches Pattern fuer vergessliches Cleanup.
                  KI vergisst nicht — Region-Lifetime erzwingt Cleanup.
Variablennamen    Nur Content-Hashes. Namen sind Lesbarkeits-Artefakte.
Error Messages    Kein Mensch sieht sie. Fehler → Abort-Code (u8).
fault_policy      Keine "ignore" / "continue_without". Entweder der
                  Pfad ist korrekt oder er wird nicht erzeugt.
SMT Timeout       Kein Timeout. Prover laeuft so lange wie noetig.
                  Unbewiesene Contracts → Graph ist UNGUELTIG.
Iteration-Limit   Keine Begrenzung auf 3 Iterationen. LLM iteriert
                  bis der Graph korrekt ist oder aufgibt.
JSON Format       Menschenlesbares Zwischenformat unnoetig.
                  KI erzeugt direkt Binaer-Nodes.
```

## Was sich verschaerft

```
VERSCHAERFT       BEGRUENDUNG
──────────────────────────────────────────────────────────────────
V-Nodes           ALLE Contracts MUESSEN bewiesen werden.
                  Kein "Timeout → Runtime-Check" Fallback.
                  Unbewiesener Contract = ungueltiger Graph.
Superoptimierung  LLVM -O3 reicht nicht. Stochastic Superoptimization:
                  Exhaustive Suche nach optimaler Instruktionssequenz.
                  Kostet Minuten statt Millisekunden — irrelevant.
Exhaustive Test   Automatische Property-Tests aus ALLEN Contracts.
                  Nicht Stichproben, sondern Bounded Model Checking.
Totale Korrektheit Graph wird NICHT kompiliert wenn ein Contract
                  nicht bewiesen ist. Keine Kompromisse.
```


## 1. Architektur v3

```
Anforderung
    │
    ▼
KI-Generator (LLM, beliebig viele Iterationen)
    │
    ▼  Binaerer FLUX-Graph (kein JSON, kein Text)
    │
Validator (Struktur + Typen + Effekte + Regionen)
    │  FAIL → zurueck zum LLM (keine Iteration-Begrenzung)
    ▼
Contract Prover (Z3, KEIN Timeout)
    │  FAIL → zurueck zum LLM
    │  ALLE Contracts muessen PROVEN sein
    ▼
Superoptimizer (exhaustive Instruktionssuche)
    │  Zeit irrelevant — findet optimale Sequenz
    ▼
MLIR → LLVM → Maschinencode
```

## 2. Node-Typen (reduziert)

```
C-Node  (Compute)     Reine Berechnung
E-Node  (Effect)      Seiteneffekte (IO, Syscall)
K-Node  (Kontroll)    Seq | Par | Branch | Loop
V-Node  (Verify)      Contract — MUSS bewiesen werden
T-Node  (Type)        Typ mit Constraints
M-Node  (Memory)      ALLOC | LOAD | STORE (Region-gebunden)
R-Node  (Region)      Speicher-Lifetime
```

Entfernt: D-Node, H-Node, P-Node, F-Node.

**Warum kein F-Node:**
F-Nodes modellierten menschliche Fehlerbehandlungsmuster (try/catch, fallback, ignore).
Eine KI erzeugt stattdessen:
- Entweder einen Graph in dem der Fehlerfall **strukturell unmoeglich** ist
  (durch Contracts bewiesen)
- Oder alternative Datenflusspfade die **beide korrekt** sind
  (via K-Node BRANCH mit V-Node auf beiden Pfaden)

Fehler sind keine Sonderfaelle — sie sind normale Pfade im Graph, die denselben
Korrektheitsbeweis durchlaufen wie der Hauptpfad.


## 3. Binaeres Format v3

```
┌───────────────────────────────────────────┐
│ FLUX v3 Binary                             │
├───────────────────────────────────────────┤
│ magic:        0x464C5833 ("FLX3")          │
│ node_count:   u32                          │
│ type_count:   u16                          │
│ region_count: u16                          │
├───────────────────────────────────────────┤
│ Type Table    [T-Nodes]                    │
│ Region Table  [R-Nodes]                    │
│ Node Table    [C|E|K|V|M-Nodes]            │
│ Edge Table    [from, to, kind]             │
│   kind: Data | Control | Effect            │
│ Contract Table [V-Nodes: formula]          │
├───────────────────────────────────────────┤
│ BLAKE3 Content Hash                        │
└───────────────────────────────────────────┘

Entfernt gegenueber v2:
  - target_hint    (Superoptimizer waehlt pro Target)
  - Debug Table    (kein D-Node)
  - proven: bool   (alle Contracts muessen proven sein)
  - check_strategy (kein Runtime-Check Fallback)
  - Fault edges    (kein F-Node, kein Fault edge-type)
```


## 4. Identitaet: Content-Hash statt Namen

```
v2:  fn calculate_tax(income: PositiveInt, rate: Probability) → PositiveInt
v3:  C-Node blake3:7a2f... { type_in: [T:a3b1, T:c4d2], type_out: T:a3b1, op: MUL }

Keine Namen. Keine Aliase. Keine Kommentare.
Identitaet = BLAKE3(type_in + type_out + op + inputs)

Gleiche Berechnung → gleicher Hash → automatische Deduplizierung.
Kein Namespace-Management, kein Import-System, kein Namenskonflikt.
```


## 5. Speichermodell — Unveraendert, aber ohne Escape-Hatch

Region-basiert wie v2, aber:

```
v2: fault_policy: "ignore" bei OOM → weiter ohne Speicher
v3: OOM ist ein Contract-Versagen. Der Graph muss BEWEISEN
    dass der Speicherbedarf innerhalb der Region-Groesse liegt.

    V-Node: max_alloc(R:frame) <= R:frame.capacity

    Beweis scheitert → Graph ungueltig → LLM erzeugt neuen Graph
    mit korrektem Speicherbedarf.
```

Regeln (unveraendert):
1. Jedes M-Node gehoert zu genau einer R-Node
2. Referenzen nur in eigene oder aeussere Region
3. Regionen werden am Scope-Ende deterministisch freigegeben


## 6. Effekt-System — Verschaerft

```
v2: E-Node mit fault_policy: "ignore" | "abort" | "continue_without"
v3: E-Node ohne Policy. Jeder E-Node hat exakt ZWEI Ausgangskanten:

    E-Node (syscall_write)
        ├── success → C-Node (naechste Berechnung)
        └── failure → C-Node (alternativer Pfad)

    BEIDE Pfade muessen:
    - Typsicher sein
    - Vollstaendig durch V-Nodes abgedeckt sein
    - Zu einem terminierten Zustand fuehren

    Es gibt keine "ignorieren" oder "abbrechen" Option.
    Jeder Pfad ist ein vollwertiger, bewiesener Graph-Pfad.
```


## 7. Contract-System — Totale Korrektheit

```
v2:                                    v3:
─────────────────────────────────────────────────────────────
PROVEN   → entfernt, 0 Overhead       PROVEN   → entfernt, 0 Overhead
DISPROVEN→ Feedback ans LLM           DISPROVEN→ Graph UNGUELTIG
TIMEOUT  → Runtime-Check (Branch)     TIMEOUT  → Prover laeuft weiter
                                                  (KEIN Timeout)

Konsequenz:
- Jedes Binary ist BEWIESENERMASSEN korrekt
  bezueglich aller spezifizierten Contracts
- Kein einziger Runtime-Check im fertigen Binary
- 0 Overhead fuer Korrektheit
- Wenn der Prover einen Contract nicht beweisen kann:
  → Graph wird verworfen
  → LLM muss einen beweisbaren Graph erzeugen
  → Oder den Contract reformulieren
```

**Prover-Strategie (Compile-Zeit irrelevant):**

```
Phase 1: Z3 / CVC5 (automatische SMT-Solver)
Phase 2: Bei Timeout → Bounded Model Checking (CBMC, KLEE)
Phase 3: Bei Timeout → Symbolische Ausfuehrung (alle Pfade)
Phase 4: Bei Timeout → Interaktiver Beweisassistent (Lean 4 / Coq)
         KI erzeugt den Beweis selbst (LLM → Lean Tactic)
Phase 5: Exhaustive Enumeration fuer endliche Domaenen
```


## 8. Superoptimierung (statt LLVM -O3)

Compile-Zeit irrelevant → optimale Instruktionssequenz per Exhaustive Search.

```
Ablauf:

1. LLVM -O3 erzeugt Baseline-Maschinencode
2. Fuer jede Funktion < 64 Instruktionen:
   Stochastic Superoptimizer (STOKE-aehnlich):
   - Zufaellige Instruktionssequenzen generieren
   - Testen ob semantisch aequivalent (via SMT)
   - Kuerzeste / schnellste Sequenz waehlen
3. Fuer groessere Funktionen:
   LLVM -O3 + zusaetzliche MLIR-Passes

Ergebnis:
- Maschinencode der manuell geschriebenem Assembler
  UEBERLEGEN sein kann
- Instruktionskombinationen die kein Mensch finden wuerde
- Kostet Minuten pro Funktion — irrelevant
```

**Beispiel: sum_array**

```
LLVM -O3 (gut):                  Superoptimized (optimal):
  vxorpd    ymm0, ymm0, ymm0      vxorpd  ymm0, ymm0, ymm0
  mov       rcx, rsi               ; Loop komplett eliminiert
  shr       rcx, 2                 ; durch rekursives AVX-Falten:
.loop:                               vaddpd  ymm0, ymm0, [rdi]
  vaddpd    ymm0, ymm0, [rdi]       vaddpd  ymm0, ymm0, [rdi+32]
  add       rdi, 32                  ...
  dec       rcx                      ; Vollstaendig entrollt fuer
  jnz       .loop                    ; bekannte Array-Groessen
  ; horizontal sum                   vhaddpd xmm0, xmm0, xmm0
  vextractf128 xmm1, ymm0, 1        ret
  vaddpd    xmm0, xmm0, xmm1
  vhaddpd   xmm0, xmm0, xmm0      ; 40% weniger Instruktionen
  ret                               ; 0 Branches
```


## 9. KI-Generierungspipeline v3

```
Anforderung
    │
    ▼
LLM erzeugt binaeren FLUX-Graph
    │     (kein JSON — direkt strukturierte Binaer-Nodes
    │      via Structured Output / Tool Calling)
    │
    ▼
┌─────────────────────────────────────────────────┐
│ VALIDATION LOOP (keine Iterations-Begrenzung)    │
│                                                  │
│   Validator:  Struktur + Typen + Effekte         │
│      │                                           │
│      ├── FAIL → detailliertes Feedback ans LLM   │
│      │          LLM erzeugt korrigierten Graph   │
│      │          → zurueck zum Validator          │
│      │                                           │
│      └── PASS ▼                                  │
│                                                  │
│   Contract Prover (KEIN Timeout):                │
│      │                                           │
│      ├── DISPROVEN → Gegenbeispiel ans LLM       │
│      │               LLM erzeugt neuen Graph     │
│      │               → zurueck zum Validator     │
│      │                                           │
│      ├── UNDECIDABLE → LLM reformuliert Contract │
│      │                 oder erzeugt Lean-Beweis  │
│      │                 → zurueck zum Prover      │
│      │                                           │
│      └── ALL PROVEN ▼                            │
│                                                  │
│   Superoptimizer:                                │
│      Exhaustive Suche nach optimaler Instruktion │
│      pro Funktion (keine Zeitbegrenzung)         │
│                                                  │
└──────────────────┬──────────────────────────────┘
                   │ Vollstaendig verifizierter,
                   │ superoptimierter Graph
                   ▼
              MLIR → LLVM → Binary
```


## 10. Vergleich v2 → v3

```
Aspekt              v2                         v3
───────────────────────────────────────────────────────────────
Node-Typen          11 (C,E,K,V,T,H,M,F,R,D,P) 7 (C,E,K,V,T,M,R)
Zwischenformat      JSON (menschenlesbar)       Binaer (nur fuer KI)
Variablennamen      Ja (intent, debug)          Nein (nur Content-Hash)
Fehlerbehandlung    F-Node + Policy             Normale Graph-Pfade
SMT Timeout         5 Sekunden                  Kein Timeout
Unbewiesene Contr.  Runtime-Check (Branch)      Graph UNGUELTIG
Iterationen LLM     Max 3                       Unbegrenzt
Compile-Zeit        ~2 Sekunden                 Minuten bis Stunden
Debug-Support       D-Node + Trace              Keiner
Optimierung         LLVM -O3                    Superoptimizer
Module              P-Node (Organisation)       Flacher Graph
Korrektheitsgarantie Teilweise                  Total
Runtime-Checks      0-N pro Binary              EXAKT 0
```


## 11. Konsequenzen

**Was dadurch moeglich wird:**

```
1. JEDES kompilierte Binary ist BEWIESENERMASSEN korrekt
   - Keine Bugs die durch Contracts abgedeckt sind
   - Null Runtime-Overhead fuer Korrektheit
   - Formal verifiziert, nicht nur getestet

2. Maschinencode ist OPTIMAL
   - Superoptimizer findet Sequenzen die kein Mensch findet
   - Besser als handgeschriebener Assembler
   - Pro Plattform individuell optimiert

3. Keine Altlasten menschlicher Programmierung
   - Kein Exception-Overhead
   - Kein Debug-Overhead
   - Kein Naming/Scoping-Overhead
   - Kein Modul-Overhead
   - Kein Abstraktions-Overhead

4. Extreme Kompaktheit
   - Hello World: nur die tatsaechlich noetigen Instruktionen
   - Kein Boilerplate, kein Framework, kein Overhead
```

**Was dadurch verloren geht:**

```
1. Kein Mensch kann den Output lesen oder debuggen
2. Kein Mensch kann den Output modifizieren
3. Compile-Zeiten von Minuten bis Stunden
4. Abhaengig von SMT-Solver-Faehigkeiten
   (manche Contracts sind prinzipiell unbeweisbar)
5. LLM muss beweisbare Graphen erzeugen koennen
   (erfordert Training auf formale Methoden)
```


## 12. Minimal-Beispiel: Hello World (v3)

```
Kein JSON. Kein Name. Kein Kommentar. Nur Struktur:

T:a1 = { kind: array, elem: u8, len: 12 }
T:a2 = { kind: u64 }
T:a3 = { kind: unit }

R:b1 = { lifetime: static }

C:c1 = { op: CONST_BYTES, val: [72,101,108,108,111,32,87,111,114,108,100,10], out: T:a1, reg: R:b1 }
C:c2 = { op: CONST, val: 1, out: T:a2 }
C:c3 = { op: CONST, val: 12, out: T:a2 }
C:c4 = { op: CONST, val: 0, out: T:a2 }

E:d1 = { op: SYSCALL_WRITE, in: [C:c2, C:c1, C:c3], out: T:a2, eff: [IO] }
  edge success → E:d2
  edge failure → C:c5

C:c5 = { op: CONST, val: 1, out: T:a2 }
E:d3 = { op: SYSCALL_EXIT, in: [C:c5], out: T:a3, eff: [PROC] }

E:d2 = { op: SYSCALL_EXIT, in: [C:c4], out: T:a3, eff: [PROC] }

V:e1 = { target: E:d1, pre: "C:c2.val == 1" }
V:e2 = { target: E:d1, pre: "C:c3.val == len(C:c1.val)" }

K:f1 = { op: SEQ, seq: [E:d1, E:d2] }
  entry: K:f1

Content-Hash: blake3:9f1a...

Anmerkung: Die obige Darstellung ist ein TEXT-RENDERING
fuer dieses Dokument. Der tatsaechliche Graph ist binaer.
Im echten System existiert kein Text.
```
