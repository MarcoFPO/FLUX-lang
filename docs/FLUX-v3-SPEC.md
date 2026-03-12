# FLUX v3 — Radikale Reduktion

## Design-Axiome

```
1. Compile-Zeit ist irrelevant           → Exhaustive Verifikation, Superoptimierung
2. Menschliche Lesbarkeit ist irrelevant → Keine Kommentare, keine Namen im Binary.
                                           LLM arbeitet mit textuellem Eingabeformat
                                           (FTL), System kompiliert zu Binaer.
3. Menschliche Hilfskonstrukte entfallen → Kein Debug, kein Exception-Handling,
                                           keine defensive Programmierung,
                                           keine Abstraktion zur Verstaendlichkeit
4. Performance der Codegenerierung       → Beliebig viele LLM-Iterationen,
   ist sekundaer                           beliebig tiefe Analyse
5. Kreativitaet ist erwuenscht           → KI soll neuartige Loesungen ERFINDEN,
                                           nicht nur bekannte Muster reproduzieren
6. Pragmatismus bei Verifikation         → Gestaffelte Prover-Strategie mit Timeouts.
                                           Unentscheidbare Contracts → Eskalation,
                                           nicht endloses Warten.
```

Vollstaendiger Vergleich v2 → v3 in Sektion 9.


## 1. Architektur v3

```
Anforderung (Mensch → LLM, out of scope)
    │
    ▼
LLM (der Programmierer)
    │
    │  SESSION_START + SESSION_SUBMIT
    │  FTL (FLUX Text Language) — strukturierter Text
    │
    ▼
┌─── FLUX-System ──────────────────────────────────┐
│                                                   │
│  FTL-Compiler (Text → Binaer, BLAKE3 berechnen)   │
│      │                                            │
│      ▼                                            │
│  Ingress (Graph empfangen, Hash verifizieren)     │
│      │                                            │
│      ▼                                            │
│  Validator (Struktur + Typen + Effekte + Regionen)│
│      │  FAIL → Feedback an LLM                    │
│      │         (Fehlerdetails + Kontext)           │
│      ▼                                            │
│  Contract Prover (gestaffelt: Z3→BMC→Lean)         │
│      │  DISPROVEN → Counterexample an LLM         │
│      │  UNDECIDABLE → Hint an LLM                 │
│      │  ALLE Contracts PROVEN ▼                   │
│      │                                            │
│  Pool / Evolution (bei OPTIMIERE/ERFINDE/ENTDECKE)│
│      │  Fitness-Feedback an LLM                   │
│      ▼                                            │
│  Compilation Gate (nur PROVEN Graphen)             │
│      │                                            │
│      ▼                                            │
│  Superoptimizer (exhaustive Instruktionssuche)     │
│      │  Zeit irrelevant — findet optimale Sequenz  │
│      ▼                                            │
│  MLIR → LLVM → Maschinencode                      │
│                                                   │
└───────────────────────────────────────────────────┘
    │
    ▼  Feedback / Binary
LLM (naechste Iteration oder SESSION_ACCEPT)
```

Siehe Sektion 12 fuer die vollstaendige Schnittstellendefinition.

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

### E-Node Effekt-Semantik

```
Jeder E-Node hat exakt ZWEI Ausgangskanten:

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

### T-Node Erweiterungen

T-Nodes unterstuetzen folgende Typ-Konstruktoren:

```
Primitiv:     integer { bits: 8|16|32|64, signed: bool }
              float   { bits: 32|64 }
              boolean
              unit

Zusammen-     struct  { fields: [(name, TypeRef)] }
gesetzt:      array   { element: TypeRef, max_length: u32,
                        constraint: optional Formula }

Summentyp:    variant { cases: [(tag: u16, payload: TypeRef)] }
              Ermoeglicht: Option<T>, Result<T,E>, Tagged Unions,
              Protokoll-Nachrichten, Baum-Strukturen.
              Jeder Branch auf einem Variant MUSS alle Cases abdecken
              (exhaustive pattern matching, durch Validator erzwungen).

Funktionstyp: fn      { params: [TypeRef], result: TypeRef,
                        effects: [EffectRef] }
              Ermoeglicht: Callbacks, Higher-Order-Funktionen,
              indirekte Aufrufe. E-Node CALL_INDIRECT nutzt fn-Typ.
              Contracts koennen ueber Funktionstypen quantifizieren.

Opaque:       opaque  { size: u32, align: u8 }
              Fuer FFI-Typen deren Struktur extern definiert ist.
```

### Alignment und Layout

```
Fuer optimale Codegenerierung definiert jeder T-Node:
  - size:   u32 (Bytes)
  - align:  u8  (Alignment in Bytes, Zweierpotenz: 1,2,4,8,16,32)
  - layout: PACKED | C_ABI | OPTIMAL

PACKED:   Kein Padding, minimaler Speicher, langsamerer Zugriff.
C_ABI:    C-kompatibles Layout (fuer FFI). Plattformabhaengig.
OPTIMAL:  FLUX waehlt optimales Layout pro Target (Felder umordnen erlaubt).
          Default fuer nicht-FFI Typen.
```


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


## 6. Contract-System — Fehlertolerante Evolution

### Paradigmenwechsel: Quarantaene statt Hinrichtung

```
v2:                     v3-alt:                  v3-NEU:
─────────────────────────────────────────────────────────────────────
PROVEN   → 0 Overhead   PROVEN   → 0 Overhead    PROVEN   → 0 Overhead
DISPROVEN→ Feedback LLM  DISPROVEN→ VERWORFEN     DISPROVEN→ INKUBATION
TIMEOUT  → Runtime-Check TIMEOUT  → kein Timeout  TIMEOUT  → INKUBATION
```

Ein Graph mit verletztem Contract wird NICHT verworfen.
Er wird in die INKUBATIONS-ZONE verschoben.
Dort wird er weiter mutiert.
Eine Folge-Mutation kann den Fehler HEILEN — und dabei etwas
hervorbringen das BESSER ist als der fehlerfreie Ausgangszustand.

```
Der Weg durch den Fehler:

Graph A (korrekt, langsam)
    │
    │ Mutation M1
    ▼
Graph A' (FEHLERHAFT — Contract V3 verletzt)
    │
    │ In klassischem Modell: → VERWORFEN. Ende.
    │ In FLUX v3:            → INKUBATION. Weiter mutieren.
    │
    │ Mutation M2
    ▼
Graph A'' (KORREKT — alle Contracts erfuellt, UND 3x schneller!)
    │
    │ Die Kombination M1+M2 hat etwas erschaffen,
    │ das durch M2 ALLEIN nie entstanden waere.
    │ M1 war der "Fehler" der noetig war.
    │
    ▼
Ergebnis: Neuartiger Algorithmus, bewiesen korrekt, ueberlegen.
```

Biologische Parallele:
```
Sichelzelleanaemie:
  Mutation 1: Haemoglobin-Gen veraendert → rote Blutzellen sichelfoermig
              → SCHAEDLICH (Anaemie, Organschaeden)
              → In "perfektem Immunsystem" → eliminiert

  ABER: Sichelzellen sind resistent gegen Malaria.
  → In Malaria-Gebieten ist die "schadhafte" Mutation ein VORTEIL.
  → Heterozygote Traeger: milde Sichelzellen + Malaria-Resistenz = OPTIMAL.

  Haette das Immunsystem die erste Mutation sofort eliminiert,
  waere die Malaria-Resistenz NIE entstanden.

FLUX-Analogie:
  Mutation 1: Sort-Algorithmus verletzt Stabilitaets-Contract
              → FEHLERHAFT (instabile Sortierung)
              → In "perfektem Immunsystem" → eliminiert

  ABER: Die instabile Variante hat ein Cache-Zugriffsmuster
  das 5x schneller ist.

  Mutation 2: Repariert Stabilitaet durch Tie-Breaking auf Originalindex
              → KORREKT + 5x SCHNELLER als der Ausgangsgraph

  Der Umweg ueber den Fehler war der SCHLUESSEL.
```


### Drei Zustaende eines Graphen

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  GESUND (alle Contracts bewiesen)                               │
│  ═══════════════════════════════                                │
│  → Kann zu Binary kompiliert werden                             │
│  → Lebt in Elite-Zone oder Toleranz-Zone                       │
│  → Wird fuer Fitness bewertet                                   │
│  → Darf gekreuzt werden                                        │
│                                                                  │
│  INKUBIERT (mindestens 1 Contract verletzt oder unbewiesen)     │
│  ══════════════════════════════════════════════════════════      │
│  → Kann NICHT zu Binary kompiliert werden                       │
│  → Lebt in der Inkubations-Zone (isoliert)                     │
│  → Wird WEITER MUTIERT (mit erhoehter Rate)                    │
│  → Wird NICHT fuer Fitness bewertet (noch nicht lauffaehig)     │
│  → Traegt Markierung: welche Contracts verletzt sind            │
│  → Traegt Markierung: wie viele Generationen in Inkubation     │
│  → Kann durch Mutation GESUND werden → aufsteigen              │
│  → Wird nach N Generationen ohne Heilung entfernt              │
│                                                                  │
│  TOT (strukturell ungueltig)                                    │
│  ═══════════════════════════                                    │
│  → Validator FAIL (kein DAG, Typ-Fehler, Region-Escape)        │
│  → Kann NICHT repariert werden durch einfache Mutation          │
│  → Wird sofort verworfen                                       │
│  → EINZIGER Zustand der zur Eliminierung fuehrt                │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘

Unterscheidung: STRUKTURELL ungueltig vs. SEMANTISCH fehlerhaft

  STRUKTURELL ungueltig:          SEMANTISCH fehlerhaft:
  Zyklen im DAG                   Contract verletzt
  Typ-Mismatch an Kanten          Falsches Ergebnis
  Region-Escape                   Out-of-Bounds (unbewiesen)
  Fehlende Inputs                 Terminierung nicht beweisbar

  → TOT (nicht reparierbar        → INKUBIERT (reparierbar
    durch Punkt-Mutation)           durch Punkt-Mutation)
```


### Inkubations-Zone: Regeln

```
1. AUFNAHME
   Graph besteht Validator (strukturell ok)
   ABER: mindestens 1 V-Node ist DISPROVEN oder UNRESOLVED
   → Graph wird in Inkubations-Zone aufgenommen
   → Markierung: {verletzt: [V3, V7], generation: 0}

2. MUTATION
   Inkubierte Graphen werden mit ERHOEHTER RATE mutiert:
   - Normal: 30% Mutation pro Generation
   - Inkubiert: 60% Mutation pro Generation
   - Mutationen werden GEZIELT in der Naehe der verletzten
     Contracts angewendet (der Prover liefert Gegenbeispiele
     die als Hinweis dienen WO der Fehler liegt)

3. RE-EVALUATION
   Nach jeder Mutation:
   - Validator: strukturell ok? → weiter. Sonst → TOT.
   - Prover: Contracts pruefen.
     → Alle PROVEN: HEILUNG! → Aufstieg in Toleranz-Zone
     → Weniger Verletzungen als vorher: Fortschritt, weiter mutieren
     → Mehr Verletzungen: Rueckschritt, aber NICHT eliminieren
       (Rueckschritte koennen spaeter zu Spruengen fuehren)

4. ALTERUNG
   Inkubations-Zaehler steigt pro Generation.
   Nach MAX_INCUBATION Generationen ohne Heilung:
   → Graph wird entfernt (Ressourcen-Limit)
   → Aber: Graph wird im ARCHIV gespeichert (nicht im aktiven Pool)
   → Archivierte Graphen koennen spaeter wiederbelebt werden
     wenn neue Mutations-Strategien verfuegbar sind

5. HEILUNG
   Wenn ein inkubierter Graph alle Contracts erfuellt:
   → Sofortige Fitness-Bewertung
   → Aufstieg in Elite/Toleranz basierend auf Fitness
   → BONUS: Geheilte Graphen erhalten Neuartigkeits-Bonus
     (der Umweg durch den Fehler hat oft ungewoehnliche
      Strukturen hervorgebracht)
```


### Pool-Architektur (revidiert)

```
┌──────────────────────────────────────────────────────────────┐
│  POOL (1000-10000 Graphen)                                    │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ ELITE-ZONE (10%)                                       │  │
│  │ Bewiesen korrekt. Beste Fitness.                       │  │
│  │ Werden nie entfernt. Eltern fuer Kreuzung.             │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ TOLERANZ-ZONE (40%)                                    │  │
│  │ Bewiesen korrekt. Neutrale Drift.                      │  │
│  │ Kein Selektionsdruck. Akkumulation.                    │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ INKUBATIONS-ZONE (30%)                                 │  │
│  │ Contract-Verletzungen. Strukturell valide.             │  │
│  │ Erhoehte Mutationsrate. Gezielte Reparatur-Mutationen.│  │
│  │ Koennen durch Mutation HEILEN → Aufstieg.             │  │
│  │ Werden nach MAX_INCUBATION archiviert.                │  │
│  │                                                        │  │
│  │ Markierungen pro Graph:                                │  │
│  │   violated_contracts: [V3, V7]                         │  │
│  │   counterexamples: [{input: [5,3,1], expected: ...}]  │  │
│  │   incubation_gen: 47                                   │  │
│  │   healing_trend: improving / stagnating / regressing   │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ PRUEF-ZONE (20%)                                       │  │
│  │ Neue Mutationen und Kreuzungen.                        │  │
│  │ Validator → Prover → Routing:                          │  │
│  │   Alle Contracts proven → Toleranz/Elite               │  │
│  │   Manche Contracts verletzt → Inkubation               │  │
│  │   Strukturell ungueltig → TOT (verwerfen)             │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ ARCHIV (unbegrenzt, persistent)                        │  │
│  │ Entfernte Inkubations-Graphen.                         │  │
│  │ Nicht im aktiven Pool. Kein Ressourcenverbrauch.       │  │
│  │ Koennen wiederbelebt werden bei neuen Strategien.      │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘

Fluss:
  Pruef-Zone ──proven──→ Toleranz-Zone ──beste──→ Elite-Zone
      │                       │
      │──verletzt──→ Inkubations-Zone ──geheilt──→ Toleranz-Zone
      │                       │
      │──strukturell──→ TOT   │──timeout──→ Archiv
         ungueltig                          (wiederbelebbar)
```


### Kompilierungs-Gate

```
NUR Graphen aus Elite-Zone oder Toleranz-Zone koennen zu Binaries
kompiliert werden. Das garantiert:

  ✓ Jedes BINARY ist bewiesen korrekt
  ✗ Nicht jeder GRAPH im Pool ist korrekt (Inkubations-Zone)

Die Inkubations-Zone ist ein LABOR, kein Produktionssystem.
Fehlerhafte Graphen existieren, werden erforscht, koennen heilen.
Aber sie verlassen das Labor erst wenn sie gesund sind.

  Pool-Ebene:  Fehlertoleranz (Inkubation, Mutation, Heilung)
  Binary-Ebene: Totale Korrektheit (nur bewiesene Graphen)
```


### Prover-Strategie (revidiert — gestaffelte Timeouts)

```
Fuer GESUNDE Graphen (→ Binary):

  Phase 1: Z3 / CVC5 (automatische SMT-Solver)
           Timeout: 60 Sekunden pro Contract
           Theorie: QF_LIA + QF_BV (entscheidbar)
           Erwartet: ~80% aller Contracts loesbar

  Phase 2: Bounded Model Checking (CBMC, KLEE)
           Timeout: 300 Sekunden (5 min)
           Fuer: Loop-Invarianten bis Bound k=100
           Erwartet: ~10% weitere Contracts

  Phase 3: Symbolische Ausfuehrung + nicht-lineare Arithmetik
           Timeout: 3600 Sekunden (1 Stunde)
           Theorie: QF_NIA, Arrays, Quantoren
           Erwartet: ~5% weitere Contracts

  Phase 4: Lean 4 / Coq (LLM-generierter Beweis)
           Timeout: kein hartes Limit, aber SESSION-Timeout
           Fuer: induktive Beweise, komplexe Quantoren
           LLM erhaelt Hint (NEEDS_INDUCTIVE_PROOF etc.)
           und liefert Lean-Taktik-Beweis

  Phase 5: Exhaustive Enumeration (endliche Domaenen)
           Timeout: abhaengig von Domaenengroesse
           Fuer: kleine Enums, bounded Arrays

  Eskalation bei Nicht-Beweis:
    → Contract bleibt UNDECIDABLE
    → Graph geht in INKUBATION (nicht verworfen!)
    → LLM erhaelt Feedback mit Hint
    → LLM kann: Contract vereinfachen, Graph umbauen,
                Lean-Beweis liefern, oder akzeptieren
                dass dieser Graph inkubiert bleibt

  Compilation Gate:
    → NUR Graphen mit ALLEN Contracts PROVEN
    → Inkubierte Graphen koennen durch Mutation/LLM-Korrektur
       zu GESUND werden und dann compiliert werden

Fuer INKUBIERTE Graphen (→ Diagnose):
  Nur Phase 1: Z3 schnell-Check (Timeout: 10s)
  Ziel: Nicht beweisen, sondern GEGENBEISPIELE finden
  Gegenbeispiele leiten gezielte Reparatur-Mutationen
  → Schnelle Diagnose, nicht vollstaendiger Beweis
```

### Contract-Sprache (formalisiert)

Contracts werden in einem SMT-LIB2-kompatiblen Subset formuliert.
Das LLM schreibt Contracts in FTL-Syntax, das System uebersetzt
sie nach SMT-LIB2 fuer den Prover.

```
Unterstuetzte Theorien:

  QF_LIA    Quantorenfreie lineare Integer-Arithmetik
            (Vergleiche, Addition, Konstanten-Multiplikation)
            → ENTSCHEIDBAR, effizient

  QF_BV     Quantorenfreie Bitvektoren
            (bitweise Operationen, Shifts, Overflow)
            → ENTSCHEIDBAR, effizient

  QF_NIA    Quantorenfreie nicht-lineare Integer-Arithmetik
            (Multiplikation von Variablen)
            → UNENTSCHEIDBAR im Allgemeinen, aber oft loesbar

  Arrays    Array-Theorie (select/store)
            → ENTSCHEIDBAR mit QF_LIA-Indizes

  Quantoren ∀ / ∃ ueber endliche Domaenen
            (z.B. "forall i in 0..n: arr[i] >= 0")
            → Entscheidbar wenn Bound bekannt

FTL-Syntax fuer Contracts:

  // Einfach (QF_LIA):
  V { target: E:d1, pre: fd.val == 1 }
  V { target: C:c3, post: result >= 0 AND result < max }

  // Array-Quantor:
  V { target: K:loop1, invariant:
      forall i in 0..snake.length: snake[i].x >= 0 }

  // Nicht-linear (QF_NIA, Prover-Phase 3):
  V { target: C:mul1, post: result == a * b AND result <= MAX_I64 }

SMT-LIB2 Uebersetzung (automatisch durch System):

  FTL: V { target: E:d1, pre: fd.val == 1 }
  SMT: (assert (= (select nodes "E:d1" "fd" "val") 1))

  FTL: V { target: K:loop1, invariant: forall i in 0..n: a[i] >= 0 }
  SMT: (assert (forall ((i Int))
         (=> (and (>= i 0) (< i n))
             (>= (select a i) 0))))
```

### Grenzen der Verifikation (explizit)

```
BEWEISBAR (interne Logik):
  - Typ-Korrektheit, Region-Safety, Effekt-Korrektheit
  - Arithmetische Constraints (Overflow, Bounds)
  - Array-Bounds, Buffer-Groessen
  - Loop-Terminierung (bei bekanntem Bound)
  - Algorithmus-Korrektheit (Sortierung, Suche, etc.)

NICHT BEWEISBAR (externe Welt):
  - Ob stdout schreibbar ist
  - Ob ein Netzwerk-Paket ankommt
  - Ob ein Dateisystem Platz hat
  - Ob ein Timer genau ist

  → E-Nodes mit externen Effekten haben IMMER zwei Pfade
    (success + failure). Contracts koennen die INTERNE Logik
    beider Pfade beweisen, aber nicht welcher Pfad genommen wird.
    Das ist kein Defizit — es ist die korrekte Modellierung
    einer nicht-deterministischen Umgebung.

SCHWIERIG (erfordert Phase 4+):
  - Terminierung unboundeter Loops
  - Nicht-lineare Arithmetik mit Variablen
  - Quantoren ueber grosse/unendliche Domaenen
  - Korrektheit von Floating-Point-Berechnungen

  → Diese Contracts erfordern LLM-assistierte Lean-Beweise
    oder werden inkubiert bis eine Loesung gefunden wird.
```


## 7. Superoptimierung (statt LLVM -O3)

Compile-Zeit irrelevant → optimale Instruktionssequenz per Exhaustive Search.

```
Ablauf (3 Stufen):

Stufe 1: LLVM -O3 Baseline (ALLE Funktionen)
  → Standard-Optimierung als Ausgangspunkt
  → Ergebnis fuer >50% der Funktionen bereits ausreichend

Stufe 2: MLIR-Level Superoptimierung (Funktionen < 200 MLIR-Ops)
  → Algebraische Vereinfachung, Fusion, Vektorisierung
  → Pattern-basiert + stochastische Suche auf MLIR-Ebene
  → Semantisch reichhaltiger als Stufe 3, groesserer Suchraum
  → Erwartet: 5-15% Verbesserung ueber LLVM -O3

Stufe 3: Instruktions-Level Superoptimierung (Funktionen < 30 Instr.)
  → STOKE-aehnlich: stochastische Suche auf Maschinencode
  → Aequivalenz-Beweis per SMT (nicht nur Testing!)
  → Fuer: Hot Loops, Kryptographie-Primitives, SIMD-Kernels
  → Realistisch bis ~30 Instruktionen (bei >30 explodiert Suchraum)
  → Erwartet: 10-40% Verbesserung fuer qualifizierte Funktionen
  → Kostet Minuten bis Stunden pro Funktion — irrelevant

Erwartete Verteilung:
  Stufe 1 only:  ~60% der Funktionen (zu gross fuer Superopt)
  Stufe 1+2:     ~30% der Funktionen (MLIR-Level Optimierung)
  Stufe 1+2+3:   ~10% der Funktionen (Hot Path, maximale Opt.)

Ergebnis:
- Hot Paths sind handgeschriebenem Assembler UEBERLEGEN
- Rest ist LLVM -O3 Qualitaet (bereits sehr gut)
- Gesamtverbesserung realistisch: 5-20% ueber reines LLVM -O3
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


## 8. Explorative Synthese — KI als Erfinder

### Das Problem mit reiner Uebersetzung

```
Bisheriges Modell:
  Anforderung → bekanntes Muster → Graph → Binary

  "Sortiere ein Array"  → Mergesort → Graph → Binary

  Die KI REPRODUZIERT. Sie erfindet nichts.
  Das ist eine Schreibmaschine, kein Ingenieur.
```

### Das neue Modell: Korrektheit als Filter, Kreativitaet als Generator

```
Neues Modell:
  Anforderung → VIELE Varianten erzeugen → filtern → bewerten → beste waehlen

  "Sortiere ein Array"  → 200 verschiedene Graphen
                         → 140 bestehen Validator
                         → 89 sind bewiesen korrekt
                         → Fitness-Bewertung
                         → Variante #67: neuartiger Hybrid aus
                           Radixsort + Insertionsort der fuer
                           diese Datenverteilung 3x schneller ist
                           als alles Bekannte

  Die KI ERFINDET. Korrektheit ist der Sicherheitsgurt, nicht das Lenkrad.
```

### Architektur der Explorativen Synthese

```
┌──────────────────────────────────────────────────────────────┐
│  PHASE 1: DIVERGENZ (viele Varianten erzeugen)               │
│                                                               │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐             │
│  │ LLM        │  │ Genetischer│  │ Constraint- │             │
│  │ Varianten  │  │ Algorithmus│  │ Relaxation  │             │
│  │            │  │ auf Graphen│  │             │             │
│  │ "Erzeuge   │  │            │  │ "Lockere    │             │
│  │  10 grund- │  │ Mutation:  │  │  Constraint │             │
│  │  verschiedene│ │ Nodes      │  │  X und suche│             │
│  │  Loesungen"│  │ ersetzen,  │  │  im groesse-│             │
│  │            │  │ Kanten     │  │  ren Raum"  │             │
│  │            │  │ umleiten,  │  │             │             │
│  │            │  │ Subgraphen │  │             │             │
│  │            │  │ kreuzen    │  │             │             │
│  └─────┬──────┘  └─────┬──────┘  └──────┬──────┘            │
│        │               │                │                    │
│        └───────────────┼────────────────┘                    │
│                        │                                     │
│                        ▼                                     │
│              Pool: N Kandidaten-Graphen                      │
│              (N = 50..10000, Zeit irrelevant)                │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 2: SELEKTION (harter Filter)                          │
│                                                               │
│  Validator:  Struktur + Typen + Effekte + Regionen           │
│              → Ungueltige Graphen verwerfen                  │
│                                                               │
│  Contract Prover:  Alle V-Nodes muessen bewiesen werden      │
│              → Unbeweisbare Graphen verwerfen                │
│                                                               │
│  Ergebnis: M korrekte Kandidaten (M <= N)                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 3: BEWERTUNG (Fitness jenseits Korrektheit)           │
│                                                               │
│  Jeder korrekte Kandidat wird bewertet nach:                 │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ Fitness-Dimensionen:                                 │    │
│  │                                                      │    │
│  │ Throughput       Instruktionen / Sekunde             │    │
│  │                  (gemessen via Sandbox-Execution)     │    │
│  │                                                      │    │
│  │ Speicher         Peak Memory / Gesamtallokation      │    │
│  │                  (statisch berechnet aus R-Nodes)     │    │
│  │                                                      │    │
│  │ Energie          Geschaetzte Instruktions-Energie     │    │
│  │                  (gewichtet nach Op-Typ und Target)   │    │
│  │                                                      │    │
│  │ Latenz           Worst-Case Ausfuehrungszeit          │    │
│  │                  (WCET-Analyse oder Messung)          │    │
│  │                                                      │    │
│  │ Graph-Groesse    Anzahl Nodes (weniger = besser)      │    │
│  │                                                      │    │
│  │ NEUARTIGKEIT     Distanz zu bekannten Loesungen      │    │
│  │                  (Structural Graph Distance)          │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                               │
│  Bewertung ist MEHRDIMENSIONAL — kein einzelner Score.       │
│  Pareto-Front: Alle Kandidaten die in mindestens einer       │
│  Dimension ungeschlagen sind, ueberleben.                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 4: EVOLUTION (optional, bei "erfinde etwas Neues")    │
│                                                               │
│  Die besten Kandidaten aus Phase 3 werden:                   │
│                                                               │
│  1. MUTIERT:  Zufaellige Aenderungen an Nodes/Kanten         │
│               (Op ersetzen, Subgraph umstrukturieren,        │
│                Parallelismus einfuegen/entfernen)            │
│                                                               │
│  2. GEKREUZT: Subgraphen zwischen zwei korrekten             │
│               Kandidaten austauschen                         │
│                                                               │
│  3. REINJIZIERT: LLM analysiert die beste Variante           │
│               und erzeugt bewusst ABWEICHENDE Variationen    │
│               ("Was waere wenn der innere Loop anders         │
│                strukturiert waere?")                          │
│                                                               │
│  → Zurueck zu Phase 2 (Filter) → Phase 3 (Bewerten)         │
│  → Wiederholen bis Konvergenz oder Abbruchkriterium          │
│                                                               │
│  Generationen: 10..1000 (Zeit irrelevant)                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
                         ▼
                  Bester Kandidat → Superoptimizer → Binary
```


### Die drei Synthese-Strategien

**Strategie 1: LLM-Divergenz (breit, schnell)**

```
Prompt an LLM:

  "Erzeuge 10 grundverschiedene FLUX-Graphen fuer:
   Sortiere ein Array von 64-bit Integers.

   Variante 1: Vergleichsbasiert
   Variante 2: Nicht-vergleichsbasiert
   Variante 3: Adaptiv (fast-sortierte Daten)
   Variante 4: Cache-optimiert (wenige Memory-Zugriffe)
   Variante 5: Minimale Branches
   Variante 6: Maximal parallel (SPAWN/JOIN)
   Variante 7: Minimaler Speicher (in-place)
   Variante 8: Hybride Strategie
   Variante 9: Unkonventionell — brich eine Regel
   Variante 10: Frei — ueberrasche"

Ergebnis: 10 strukturell verschiedene Graphen,
          die dasselbe Interface und dieselben Contracts erfuellen.
```

**Strategie 2: Genetische Graph-Evolution (tief, langsam)**

```
Population:
  100 korrekte Graphen (aus LLM-Divergenz oder Mutation)

Mutation-Operatoren auf Graph-Ebene:
  ┌────────────────────────────────────────────────────────┐
  │ NODE_REPLACE   Ersetze C-Node(ADD) durch C-Node(SUB)  │
  │                + Kompensation (V-Node muss halten)     │
  │                                                        │
  │ EDGE_REWIRE    Verbinde Input eines Nodes mit einem    │
  │                anderen Node gleichen Typs              │
  │                                                        │
  │ SUBGRAPH_SWAP  Tausche Subgraph A gegen Subgraph B    │
  │                aus einem anderen Kandidaten            │
  │                                                        │
  │ LOOP_TRANSFORM Aendere Loop-Strategie:                 │
  │                Iterativ ↔ Rekursiv ↔ Entrollt          │
  │                ↔ Vektorisiert ↔ Parallel               │
  │                                                        │
  │ BRANCH_ELIM    Ersetze BRANCH durch branchless         │
  │                Arithmetik (CMOV-Pattern)               │
  │                                                        │
  │ PAR_INSERT     Fuege Parallelismus ein wo              │
  │                keine Datenabhaengigkeit besteht        │
  │                                                        │
  │ REGION_MERGE   Vereinige zwei Regionen                 │
  │                (weniger alloc/free, groesserer Block)   │
  │                                                        │
  │ CONST_FOLD     Ersetze berechenbare Subgraphen         │
  │                durch ihre Ergebnisse                   │
  │                                                        │
  │ NOVEL_INSERT   Fuege einen zufaelligen, typkorrekten   │
  │                Subgraphen ein und pruefe ob             │
  │                die Contracts noch gelten               │
  └────────────────────────────────────────────────────────┘

Selection:  NSGA-II (Multi-Objective)
            Pareto-Front aus Throughput × Speicher × Neuartigkeit

Generationen: bis Konvergenz (typisch 50-500)
```

**Strategie 3: Constraint-Relaxation (gezielt, explorativ)**

```
Ausgangslage:
  "Verbessere die Bildkompression"

  Bestehender Graph: DCT-basiert (wie JPEG)
  Bestehende Contracts:
    V1: output.size < input.size * ratio
    V2: decompress(compress(input)) == input  (verlustfrei)

Schritt 1: Relaxation
  V2 aendern: decompress(compress(input)) ≈ input  (Distanz < epsilon)
  → Groesserer Loesungsraum (verlustbehaftet erlaubt)

Schritt 2: Exploration
  LLM generiert Varianten im erweiterten Raum:
  - Wavelet-basiert statt DCT
  - Fraktale Kompression
  - Neuronale Kompression (Lookup-Table aus Training)
  - Hybrid: grobe DCT + feine Wavelet-Korrektur
  - UNBEKANNT: etwas das kein Name hat

Schritt 3: Re-Verifikation
  Alle Varianten muessen die relaxierten Contracts erfuellen.
  Zusaetzlich: empirische Qualitaetsmessung (SSIM, PSNR)

Schritt 4: Pareto-Selektion
  Kompressionsrate × Qualitaet × Geschwindigkeit
  → Variante #4 (Hybrid) ist auf der Pareto-Front
  → Variante #5 (UNBEKANNT) hat besten Qualitaets/Groessen-Tradeoff
```


### Neuartigkeits-Metrik

Wie misst man ob eine Loesung "neu" ist?

```
Structural Graph Distance (SGD):

  Gegeben: Graph G_neu und eine Datenbank bekannter Graphen {G_1, ..., G_k}

  SGD(G_neu) = min_i( EditDistance(G_neu, G_i) )

  EditDistance zaehlt:
    - Nodes hinzugefuegt/entfernt
    - Ops geaendert (ADD→MUL = 1 Edit)
    - Kanten umgeleitet
    - Subgraphen ersetzt

  Hoher SGD = hohe Neuartigkeit = bevorzugt in Pareto-Selektion

  Die Datenbank waechst mit jeder Synthese:
  → Jedes akzeptierte Binary wird zum Referenzpunkt
  → System wird ueber Zeit kreativer, weil es sich von
     immer mehr bekannten Loesungen entfernen muss
```


### Konkretes Beispiel: "Verbessere sort()"

```
Anforderung:
  "Verbessere die Sortierung von 64-bit Integer-Arrays.
   Erfinde dafuer etwas Neues."

Contracts (unverhandelbar):
  V1: result ist aufsteigend sortiert
  V2: result ist Permutation von input
  V3: Terminierung fuer alle endlichen Inputs

Phase 1 — Divergenz:
  LLM erzeugt 50 Varianten + Genetischer Algo erzeugt 150 Mutationen
  = 200 Kandidaten

Phase 2 — Selektion:
  187 bestehen Validator
  134 sind bewiesen korrekt (V1, V2, V3 alle proven)
  66 verworfen (nicht beweisbar oder inkorrekt)

Phase 3 — Bewertung (Sandbox-Execution, 10M Elemente):

  Kandidat   Typ                    Throughput   Speicher  SGD
  ────────────────────────────────────────────────────────────
  #012       Mergesort (Standard)   480 MB/s     O(n)      0.0
  #023       Radixsort              890 MB/s     O(n)      3.2
  #041       Introsort              510 MB/s     O(log n)  2.1
  #067       ??? (KI-Erfindung)     1120 MB/s    O(√n)     8.7  ← NEU
  #089       Parallel Mergesort     1450 MB/s    O(n)      4.1
  #134       ??? (KI-Erfindung)     920 MB/s     O(1)      9.3  ← NEU

  Pareto-Front: {#067, #089, #134}

Phase 4 — Analyse von #067 (beste Neuartigkeit + Throughput):

  Was hat die KI erfunden?

  Graph-Struktur (automatisch rekonstruiert):
  1. Grobe Partitionierung durch Radix auf die oberen 16 Bits
     (256 Buckets, cache-line-aligned, branchless Verteilung)
  2. Pro Bucket: Insertionsort wenn n < 32, sonst Bitonic Sort
  3. Merge-Phase eliminiert: Buckets sind bereits geordnet
     durch die Radix-Partitionierung
  4. SIMD-Nutzung: 4-Element Sorting Networks via VMIN/VMAX
     innerhalb der Insertionsort-Phase

  Kein bekannter Algorithmus. Hybrid aus Radix + Bitonic + SIMD-Networks.
  Bewiesen korrekt. 2.3x schneller als std::sort fuer gleichverteilte Daten.

  Variante #067 → Superoptimizer → Binary
```


### Wachsende Wissensbasis

```
Jedes akzeptierte Binary erweitert die Wissensbasis:

┌──────────────────────────────────────────────────────────────┐
│  GRAPH REPOSITORY (Content-Addressiert)                      │
│                                                               │
│  Jeder Graph wird gespeichert mit:                           │
│  - Content-Hash (Identitaet)                                 │
│  - Bewiesene Contracts (was er garantiert)                   │
│  - Fitness-Profil (Throughput, Speicher, Latenz, ...)        │
│  - Neuartigkeits-Score zum Zeitpunkt der Erzeugung          │
│  - Abstammung (aus welcher Mutation/Kreuzung entstanden)     │
│                                                               │
│  Nutzen:                                                     │
│  - Subgraphen koennen in neuen Synthesen wiederverwendet     │
│    werden (bewiesene Korrektheit bleibt erhalten)            │
│  - Neuartigkeits-Metrik wird schaerfer (mehr Referenzpunkte) │
│  - Genetische Kreuzung nutzt bewaehrte Subgraphen            │
│  - KI lernt welche Strukturen in welchen Kontexten           │
│    ueberlegen sind                                           │
│                                                               │
│  Wachstum:                                                   │
│  Tag 1:     ~100 Referenz-Graphen (Standardalgorithmen)     │
│  Monat 1:   ~5.000 (erste kreative Varianten)               │
│  Jahr 1:    ~500.000 (eigenes Oekosystem)                   │
│  Langfristig: Millionen — eigene "Algorithmische DNA"        │
└──────────────────────────────────────────────────────────────┘
```


### Biologisches Modell: Kimuras Neutrale Theorie

Die explorative Synthese folgt Kimuras "Neutral Theory of Molecular
Evolution" (1968): Die meisten Mutationen sind neutral, aber sie sind
das Rohmaterial aus dem Innovation entsteht.

```
Klassischer GA:              Biologisches Modell (FLUX):
─────────────────            ─────────────────────────────
Mutation → bewerten          Mutation → Contract-Check
→ nur Beste ueberleben       → wenn nicht schaedlich: BEHALTEN
→ Vielfalt sinkt             → Vielfalt bleibt erhalten
→ lokales Optimum            → neutraler Drift akkumuliert
→ keine Innovation           → Kombination → qualitativer Sprung
```

**Kernprinzip:** V-Nodes sind Diagnose, nicht Todesurteil.
Fehlerhafte Graphen → INKUBATION (Sektion 6). Fehler als Rohmaterial.
Nur auf Binary-Ebene: totale Korrektheit (Compilation Gate).

**5 Phasen:**

```
Phase 0: GENESIS        LLM erzeugt 50-100 Ausgangs-Graphen
Phase 1: NEUTRALE DRIFT Kleine Mutationen ohne Performance-Effekt.
         (Gen 1-50)     Pool diversifiziert sich strukturell.
Phase 2: WUCHERUNG      Groessere Mutationen, Subgraphen wachsen.
         (Gen 50-200)   Manche verletzen Contracts → Inkubation.
                        Material fuer spaetere Innovation.
Phase 3: EMERGENZ       Akkumulierte Mutationen interagieren.
         (Gen 200+)     Qualitativer Sprung. Inkubierte Graphen
                        heilen UND sind ueberlegen (Sichelzell-Prinzip).
Phase 4: RADIATION      Emergenter Graph → Spezialisierung in Nischen
                        (kleine Arrays, fast-sortiert, minimal-Speicher).
```


Pool-Architektur und Parameter: siehe Sektion 6 (Pool-Architektur).
Die Phasen nutzen die dort definierten Zonen (Elite, Toleranz, Inkubation, Pruef, Archiv).


### Anforderungstypen fuer Kreativitaet

```
Typ 1: UEBERSETZE (keine Kreativitaet noetig)
  "Sortiere ein Array mit Mergesort"
  → Genau ein bekannter Algorithmus → direkte Synthese

Typ 2: OPTIMIERE (gerichtete Kreativitaet)
  "Sortiere ein Array moeglichst schnell"
  → Viele bekannte + neue Varianten → Pareto-Selektion

Typ 3: ERFINDE (maximale Kreativitaet)
  "Verbessere die Sortierung. Erfinde etwas Neues."
  → Volle explorative Synthese mit Neuartigkeits-Bonus
  → Genetische Evolution ueber Generationen
  → Constraint-Relaxation erlaubt

Typ 4: ENTDECKE (offene Exploration)
  "Finde eine Berechnung die Eigenschaft X hat."
  → Nur Contracts definiert, kein Algorithmus vorgegeben
  → Reine Suche im Graphen-Raum
  → Kann fundamental neue Algorithmen hervorbringen
```


## 9. Vergleich v2 → v3

```
Aspekt              v2                         v3
───────────────────────────────────────────────────────────────
Node-Typen          11                         7 (+Summentyp, +Funktionstyp in T-Node)
LLM-Eingabe         JSON (menschenlesbar)      FTL (Text, LLM-optimiert)
Internes Format     JSON                       Binaer (BLAKE3, vom System berechnet)
Variablennamen      Ja                         Nein (Content-Hash)
Fehlerbehandlung    F-Node + Policy            Normale Graph-Pfade
SMT Timeout         5 Sekunden                 Gestaffelt: 60s/300s/3600s/unbegrenzt
Unbewiesene Contr.  Runtime-Check (Branch)     Inkubation + Eskalation
Contract-Sprache    Informell                  SMT-LIB2-Subset (formalisiert)
Iterationen LLM     Max 3                      Unbegrenzt
Compile-Zeit        ~2 Sekunden                Minuten bis Stunden
Debug-Support       D-Node + Trace             Keiner
Optimierung         LLVM -O3                   3-stufig: LLVM + MLIR-Superopt + STOKE
Module              P-Node (Organisation)      Flacher Graph
Korrektheitsgarantie Teilweise                 Total (fuer bewiesene Contracts)
Runtime-Checks      0-N pro Binary             EXAKT 0
Kreativitaet        Keine (1:1 Uebersetzung)   Explorative Synthese
Varianten           1 Graph pro Anforderung    50-10000 Kandidaten
Wissensbasis        Keine                      Wachsendes Graph Repository
LLM-Schnittstelle   Nicht definiert            FTL + JSON-Feedback (Sektion 12)
Feedback            Text (menschenlesbar)      JSON (maschinenlesbar, relative Metriken)
Concurrency         Nicht definiert            K:Par + atomare Ops (Sektion 13)
FFI                 Nicht definiert            Extern-Deklarationen + Trust (Sektion 14)
MLIR-Lowering       Nicht definiert            5-Phasen-Pipeline (Sektion 15)
Scope               Nicht definiert            Klar abgegrenzt (Sektion 16)
```


## 10. Konsequenzen

**Was dadurch moeglich wird:**

```
1. JEDES kompilierte Binary ist BEWIESENERMASSEN korrekt
   - Keine Bugs die durch Contracts abgedeckt sind
   - Null Runtime-Overhead fuer Korrektheit
   - Formal verifiziert, nicht nur getestet

2. Hot-Path-Code ist OPTIMAL, Rest ist LLVM -O3 Qualitaet
   - Superoptimizer fuer kleine Funktionen (<30 Instr.)
   - 5-20% Gesamtverbesserung ueber reines LLVM -O3
   - Pro Plattform individuell optimiert

3. KI kann NEUARTIGE ALGORITHMEN erfinden
   - Nicht limitiert auf bekannte Patterns
   - Explorative Synthese mit genetischer Evolution
   - Constraint-Relaxation oeffnet neue Loesungsraeume
   - Korrektheit ist garantiert — Kreativitaet ist frei

4. Wachsendes Oekosystem eigener Erfindungen
   - Graph Repository als "algorithmische DNA"
   - Jede Synthese erweitert die Wissensbasis
   - Neuartigkeits-Metrik verhindert Stagnation
   - System wird ueber Zeit kreativer

5. Extreme Kompaktheit
   - Kein Boilerplate, kein Framework, kein Overhead
   - Keine menschlichen Abstraktionen, nur Berechnung
```

**Was dadurch verloren geht:**

```
1. Kein Mensch kann den Output lesen oder debuggen
2. Kein Mensch kann den Output modifizieren
3. Compile-Zeiten von Minuten bis Stunden (oder Tagen bei ENTDECKE)
4. Abhaengig von SMT-Solver-Faehigkeiten (gestaffelt, aber real)
5. Erfundene Algorithmen sind nicht erklaerbar
   (sie funktionieren beweisbar, aber niemand versteht warum)
6. Spezialisiertes LLM-Training noetig (kein Trainingskorpus existiert)
7. Komplexe Programme (>1000 Nodes) erfordern Graph-Partitionierung
   (noch nicht in v3 spezifiziert)
```


## 11. Minimal-Beispiel: Hello World (FTL)

Das LLM erzeugt folgenden FTL-Text:

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
                       effects: [IO], success: E:d2, failure: E:d3 }
E:d2 = syscall_exit { inputs: [C:c4], type: T:a3, effects: [PROC] }
E:d3 = syscall_exit { inputs: [C:c5], type: T:a3, effects: [PROC] }

V:e1 = contract { target: E:d1, pre: C:c2.val == 1 }
V:e2 = contract { target: E:d1, pre: C:c3.val == 12 }

K:f1 = seq { steps: [E:d1, E:d2] }
entry: K:f1
```

Das System kompiliert diesen FTL-Text zu:
1. Binaerer Graph mit BLAKE3-Hash (automatisch)
2. Validator prueft Struktur → PASS
3. Prover beweist V:e1 und V:e2 → PROVEN (trivial, 0.1ms)
4. Superoptimizer → optimaler Maschinencode (137 Bytes x86-64)


## 12. LLM→FLUX Schnittstelle — Das LLM als Programmierer

### Grundprinzip

```
Das LLM ERSETZT den menschlichen Programmierer vollstaendig.

Mensch → LLM:       Out of Scope (natuerliche Sprache, Prompt, etc.)
LLM → FLUX-System:  IN Scope — FTL (FLUX Text Language)
FLUX → LLM:         IN Scope — strukturiertes JSON-Feedback
```

Die Interaktion Mensch→LLM ist nicht Teil der FLUX-Spezifikation.
FLUX definiert ausschliesslich, was das LLM liefert, wie es liefert,
und welches Feedback es zurueckbekommt.

**Zentrale Design-Entscheidung:** Das LLM erzeugt KEIN Binaerformat
und berechnet KEINE kryptographischen Hashes. LLMs sind Token-basierte
Textgeneratoren — binaere Formate und Hash-Berechnung liegen fundamental
ausserhalb ihrer Architektur. Stattdessen:

```
LLM erzeugt:     FTL (strukturierter Text, LLM-optimiert)
System erzeugt:  Binaeren Graph + BLAKE3-Hashes (deterministisch)
```


### FTL — FLUX Text Language

FTL ist das textuelle Eingabeformat, das LLMs erzeugen.
Es wird vom FLUX-System deterministisch in den binaeren Graph
kompiliert. FTL ist NICHT fuer Menschen optimiert, aber
fuer Token-basierte Textgeneratoren.

```
Design-Prinzipien:
  - Keine Variablennamen — nur IDs (T:a1, C:c1, E:d1, etc.)
  - Keine Kommentare, keine Whitespace-Semantik
  - Eindeutige Syntax, minimale Ambiguitaet
  - Jede Zeile ist ein Statement (kein Multiline)
  - IDs sind lokal zum Graph (System berechnet Content-Hashes)
```

**FTL-Grammatik (Auszug):**

```
// Typen
T:a1 = integer { bits: 64, signed: false }
T:a2 = array { element: T:a1, max_length: 1024 }
T:a3 = struct { fields: [x: T:a1, y: T:a1] }
T:a4 = variant { cases: [NONE: unit, SOME: T:a1] }
T:a5 = fn { params: [T:a1, T:a2], result: T:a3, effects: [IO] }

// Regionen
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }

// Compute-Nodes
C:c1 = const { value: 42, type: T:a1, region: R:b1 }
C:c2 = add { inputs: [C:c1, C:c3], type: T:a1 }
C:c3 = call_pure { target: "sort", inputs: [C:c5], type: T:a2 }

// Effect-Nodes (immer 2 Ausgaenge)
E:d1 = syscall_write { inputs: [C:c1, C:c2, C:c3],
                        type: T:a1, effects: [IO],
                        success: E:d2, failure: C:c4 }

// Kontroll-Nodes
K:f1 = seq { steps: [E:d1, E:d2, E:d3] }
K:f2 = branch { condition: C:c5, true: K:f1, false: K:f3 }
K:f3 = loop { condition: C:c6, body: K:f4,
              state: C:c7, state_type: T:a3 }
K:f4 = par { branches: [K:f1, K:f5], sync: BARRIER }

// Contracts (SMT-LIB2-kompatibel, siehe Sektion 6)
V:e1 = contract { target: E:d1, pre: C:c1.val == 1 }
V:e2 = contract { target: K:f3, invariant:
                  forall i in 0..state.length: state[i] >= 0 }

// Memory-Nodes
M:g1 = alloc { type: T:a2, region: R:b2 }
M:g2 = load { source: M:g1, index: C:c1, type: T:a1 }
M:g3 = store { target: M:g1, index: C:c1, value: C:c2 }

// Entry-Point
entry: K:f1
```

**FTL → Binaer Kompilierung (automatisch):**

```
LLM erzeugt FTL
    │
    ▼
FTL-Parser (Syntax-Check, deterministisch)
    │
    ▼
ID-Aufloesung (lokale IDs → interne Referenzen)
    │
    ▼
BLAKE3-Berechnung (Content-Hash pro Node, pro Graph)
    │
    ▼
Binaerer FLUX-Graph (identisch zu Sektion 3)
    │
    ▼
Validator (wie bisher)
```


### Architektur

```
┌─────────────┐         ┌──────────────────────────────────┐
│             │         │         FLUX-System               │
│    LLM      │         │                                  │
│ (Programm-  │  ──────▶│  FTL-Compiler (Text → Binaer)    │
│  ierer)     │  FTL    │      │ + BLAKE3-Berechnung       │
│             │         │      ▼                           │
│             │         │  Validator                       │
│             │  ◀──────│      │                           │
│             │  JSON   │      ▼                           │
│             │ Feedback│  Contract Prover (gestaffelt)     │
│             │         │      │                           │
│             │  ◀──────│      ▼                           │
│             │  JSON   │  Pool / Evolution                │
│             │ Feedback│      │                           │
│             │         │      ▼                           │
│             │         │  Compilation Gate                │
│             │         │      │                           │
│             │         │      ▼                           │
│             │         │  Superoptimizer → Binary         │
└─────────────┘         └──────────────────────────────────┘
```


### Graph-Submission

Das LLM liefert FTL-Text an das System.
Jede Submission enthaelt:

```
{
  "session_id": "uuid",
  "submission_type": "TRANSLATE | OPTIMIZE | INVENT | DISCOVER",
  "target_arch": "ANY | X86_64 | AARCH64 | RISCV64 | WASM",
  "graphs": [
    {
      "ftl": "T:a1 = integer { bits: 8 }\n...\nentry: K:f1",
      "parent_ref": null | "blake3:...",
      "generation": 0,
      "mutation_log": []
    }
  ],
  "constraints": {
    "max_binary_size": null | 65536,
    "max_memory": null | 1048576,
    "min_throughput": null | 1000.0
  }
}
```

Das System kompiliert den FTL-Text zu binaeren Graphen,
berechnet BLAKE3-Hashes und fuehrt die Pipeline aus.
Bei FTL-Syntaxfehlern wird sofort Feedback zurueckgegeben
(bevor der Validator ueberhaupt laeuft).


### Feedback-Protokoll

Feedback ist JSON — optimal fuer LLM-Verarbeitung.

```json
{
  "status": "VALIDATION_FAIL | CONTRACT_FAIL | ACCEPTED
             | INCUBATED | COMPILED",
  "graph_hash": "blake3:9f1a...",

  // Bei VALIDATION_FAIL:
  "validation_errors": [
    {
      "error_code": 1001,
      "node_id": "E:d1",
      "violation": "TYPE_MISMATCH",
      "message": "E:d1 input[0] expects T:a1 (u64), got T:a2 (array)",
      "context": {
        "expected_type": "T:a1",
        "actual_type": "T:a2",
        "suggestion": "Add C-Node with op:ARRAY_LENGTH before E:d1"
      }
    }
  ],

  // Bei CONTRACT_FAIL:
  "contract_results": [
    {
      "contract_id": "V:e1",
      "status": "DISPROVEN",
      "counterexample": {
        "bindings": { "C:c1": 0, "C:c2": -5 },
        "trace": ["K:f1", "C:c2", "E:d1"],
        "explanation": "When C:c2 = -5, pre-condition fd >= 0 fails"
      }
    },
    {
      "contract_id": "V:e2",
      "status": "UNDECIDABLE",
      "prover_phase": 3,
      "hint": "NEEDS_INDUCTIVE_PROOF",
      "suggestion": "Loop invariant requires induction on array length"
    }
  ],

  // Bei INCUBATED:
  "incubation_info": {
    "pool_position": "TOLERANCE",
    "fitness": {
      "throughput_rank": 3,
      "throughput_percentile": 0.95,
      "memory_rank": 12,
      "latency_rank": 1,
      "novelty_sgd": 0.73
    },
    "pool_size": 134,
    "suggestion": "Graph is fast but V:e2 unproven. Try simplifying loop."
  },

  // Bei COMPILED:
  "compilation_result": {
    "binary_hash": "blake3:...",
    "binary_size": 4892,
    "targets": {
      "x86_64": { "instructions": 47, "cycles_estimate": 23,
                  "superopt_improvement_pct": 12.5 },
      "aarch64": { "instructions": 52, "cycles_estimate": 28,
                   "superopt_improvement_pct": 8.1 }
    }
  }
}
```

**Anmerkung:** Fitness-Werte werden als RELATIVE Metriken
(Rang, Perzentil) statt absolute Werte (f64) zurueckgegeben.
LLMs koennen relative Positionen besser interpretieren als
rohe Throughput-Zahlen.


### Feedback-Loop

```
LLM erzeugt FTL-Graph(en)
    │
    ▼
FLUX: FTL-Compiler
    │
    ├── SYNTAX_ERROR → sofortiges Feedback
    │   (Zeile, Position, erwartetes Token)
    │   LLM korrigiert FTL → resubmit
    │
    └── OK → Validator
              │
              ├── FAIL → Feedback mit Fehlerdetails + Suggestions
              │   LLM korrigiert → neuer Graph (parent_ref gesetzt)
              │
              └── PASS → Contract Prover (gestuft, Sek. 6)
                          │
                          ├── DISPROVEN → Counterexample + Erklaerung
                          │   LLM sieht konkrete Werte die zum
                          │   Widerspruch fuehren → korrigiert
                          │
                          ├── UNDECIDABLE → Hint + Suggestion
                          │   LLM kann:
                          │     a) Contract vereinfachen
                          │     b) Graph umbauen
                          │     c) Lean-Beweis als FTL-Annotation liefern
                          │     d) Akzeptieren → Graph wird inkubiert
                          │
                          └── ALL PROVEN
                              │
                              ├── TRANSLATE → Superoptimizer → Binary
                              │
                              └── OPTIMIZE/INVENT/DISCOVER
                                  → Pool → Evolution → Fitness-Feedback
                                  → LLM liefert weitere Varianten

Keine Iterations-Begrenzung. Der Loop laeuft bis:
  - Mindestens 1 Graph COMPILED ist, oder
  - Das LLM signalisiert: ABORT
```


### LLM-Capabilities

```
MUSS:
  - Syntaktisch korrektes FTL erzeugen
  - Auf Validation-Feedback reagieren und korrigieren
  - Auf Counterexamples reagieren und Contracts erfuellen
  - Typ-korrekte Graphen erzeugen (Typ-Fehler erkennen)

KANN:
  - Mehrere Graphen gleichzeitig liefern (Batch)
  - Lean-Beweise als FTL-Annotationen liefern
  - Mutationen gezielt auf Fitness-Feedback anwenden
  - Graph-Fragmente aus dem Repository referenzieren
  - Contracts in SMT-LIB2-kompatiblem Subset formulieren

MUSS NICHT (System uebernimmt):
  - BLAKE3-Hashes berechnen
  - Binaeres Format erzeugen
  - Optimale Instruktionen waehlen
  - Speicher-Layout bestimmen
```


### Session-Management

```json
// SESSION_START
{ "action": "START",
  "type": "TRANSLATE | OPTIMIZE | INVENT | DISCOVER",
  "target": "ANY | X86_64 | AARCH64 | RISCV64 | WASM",
  "constraints": { "max_binary_size": 65536 } }

// SESSION_SUBMIT (1..N Graphen als FTL)
{ "action": "SUBMIT",
  "graphs": [{ "ftl": "...", "parent_ref": null }] }

// SESSION_QUERY (Pool-Status abfragen)
{ "action": "QUERY", "query": "POOL_STATUS" }
// Response:
{ "pool_size": 134, "elite": 13, "tolerance": 54,
  "incubation": 40, "probe": 27,
  "best_fitness": { "throughput_rank": 1, ... },
  "pareto_front": ["blake3:...", "blake3:..."] }

// SESSION_ABORT
{ "action": "ABORT", "reason": "complexity_exceeds_capability" }

// SESSION_ACCEPT
{ "action": "ACCEPT", "graph_hash": "blake3:..." }
```


## 13. Concurrency-Modell

### K-Node:Par — Parallele Ausfuehrung

```
K-Node:Par definiert parallele Ausfuehrungspfade mit
expliziter Synchronisation und Memory-Ordering.

K:f1 = par {
  branches: [K:f2, K:f3, K:f4],
  sync: BARRIER | NONE,
  memory_order: SEQ_CST | ACQUIRE_RELEASE | RELAXED
}
```

### Memory-Ordering

```
SEQ_CST (Default, sicher):
  Sequentielle Konsistenz. Alle Threads sehen Operationen
  in derselben Reihenfolge. Einfach zu verifizieren.
  Performance-Overhead: ~10-20% gegenueber RELAXED.

ACQUIRE_RELEASE:
  Producer-Consumer-Pattern. Store mit RELEASE,
  Load mit ACQUIRE. Contracts koennen Happens-Before
  beweisen. Fuer: Message-Passing, Lock-Free Queues.

RELAXED:
  Keine Ordering-Garantien. NUR fuer unabhaengige
  Berechnungen ohne geteilten Speicher.
  Contract MUSS beweisen: Branches teilen keine M-Nodes.
```

### Geteilter Speicher

```
Parallele Branches duerfen NICHT auf dieselben M-Nodes
zugreifen, ausser ueber explizite Sync-Nodes:

S-Node (Sync):  Nicht als eigener Node-Typ, sondern als
                 spezielle C-Node Operationen:

  C:s1 = atomic_load  { source: M:g1, order: ACQUIRE, type: T:a1 }
  C:s2 = atomic_store { target: M:g1, value: C:c1, order: RELEASE }
  C:s3 = atomic_cas   { target: M:g1, expected: C:c1,
                         desired: C:c2, order: SEQ_CST,
                         success: K:f1, failure: K:f2 }

Contract fuer Data-Race-Freedom:
  V:race = contract { target: K:f1_par,
    invariant: forall (b1, b2) in branches:
      shared_mnodes(b1, b2) == {} OR
      all_accesses_atomic(b1, b2) }
```

### Einschraenkungen (bewusst)

```
NICHT unterstuetzt in v3:
  - OS-Threads (zu plattformspezifisch)
  - Gruene Threads / Coroutines (Scheduler noetig)
  - Async/Await (menschliches Abstraktionsmuster)

Unterstuetzt:
  - Fork-Join Parallelismus (K:Par + BARRIER)
  - Lock-Free Algorithmen (atomic ops + Contracts)
  - SIMD/Vektorisierung (durch Superoptimizer)
  - Daten-Parallelismus (K:Par ueber Array-Partitionen)

Begruendung: FLUX erzeugt Binaries, keine Laufzeitsysteme.
Komplexe Concurrency (Actor, CSP, Async) gehoert in ein
Runtime-Layer, das FLUX NUTZT, aber nicht DEFINIERT.
```


## 14. FFI — Interaktion mit der Aussenwelt

### Warum FFI noetig ist

```
Kein reales Programm existiert in Isolation.
FLUX-Binaries muessen mit Betriebssystem-APIs, C-Libraries,
Hardware-Treibern und existierenden Codebases interagieren.

Ohne FFI waere FLUX auf rohe Syscalls beschraenkt — das
reicht fuer algorithmische Kernels, aber nicht fuer
Anwendungen die Netzwerk, Dateisystem, GUI oder
Datenbanken nutzen.
```

### FFI-Mechanismus

```
Extern-Deklarationen in FTL:

// Externe Funktion deklarieren (C ABI)
X:ext1 = extern { name: "memcpy", abi: C,
                   params: [T:ptr, T:ptr, T:size_t],
                   result: T:ptr, effects: [MEM] }

// Externe Funktion aufrufen (wie E-Node)
E:d5 = call_extern { target: X:ext1,
                      inputs: [C:c1, C:c2, C:c3],
                      type: T:ptr, effects: [MEM],
                      success: K:f2, failure: K:f3 }

// Opaque Typ fuer externe Strukturen
T:ext_struct = opaque { size: 128, align: 8 }
```

### Verifikation von FFI

```
Extern-Aufrufe sind NICHT formal verifizierbar
(das externe Verhalten ist unbekannt). Stattdessen:

1. Der AUFRUF wird verifiziert:
   - Parameter-Typen stimmen mit Deklaration ueberein
   - Effekte sind deklariert
   - Beide Pfade (success/failure) sind abgedeckt

2. Die ANNAHMEN werden als Contracts formuliert:
   V:ffi1 = contract { target: E:d5, trust: EXTERN,
     assume: result != NULL AND result == C:c1,
     post: region_valid(R:b2) }

   trust: EXTERN markiert den Contract als ANNAHME,
   nicht als BEWEIS. Das Binary wird kompiliert, aber
   mit dem Vermerk "enthaelt unverifizierten FFI-Trust".

3. Statistik im Compilation-Result:
   "verified_contracts": 15,
   "trusted_contracts": 2,    // FFI-Annahmen
   "trust_boundary": ["X:ext1", "X:ext2"]
```

### Scope der FFI

```
Unterstuetzt:
  - C ABI (cdecl, System V AMD64, AAPCS64)
  - POSIX Syscalls (direkt, ohne libc)
  - Statisches Linking gegen .a / .o Dateien

Nicht unterstuetzt (out of scope fuer v3):
  - C++ ABI (Name Mangling, Exceptions, RTTI)
  - Dynamisches Linking (.so / .dll)
  - Scripting-Language FFI (Python, JS)
```


## 15. FLUX→MLIR Lowering

### Dialekt-Mapping

```
FLUX Node-Typ    → MLIR Dialekt        Anmerkung
──────────────────────────────────────────────────────────
C-Node (Compute) → arith, math         ADD→arith.addi, MUL→arith.muli
                                        Floating-Point→arith.addf etc.

E-Node (Effect)  → func.call + scf.if  Call + Branch auf success/failure
                   custom FLUX-Dialekt  Syscalls als custom ops

K-Node:Seq       → (implizit)          Sequenz = MLIR Block-Reihenfolge
K-Node:Branch    → scf.if / scf.index_switch
K-Node:Loop      → scf.while / scf.for
K-Node:Par       → async.execute       Oder custom parallel-Dialekt

V-Node (Verify)  → (entfernt)          Contracts sind vor MLIR bewiesen,
                                        tauchen in MLIR nicht auf

T-Node (Type)    → builtin types       i8, i16, i32, i64, f32, f64,
                   memref               memref<NxT> fuer Arrays

M-Node (Memory)  → memref.alloc        ALLOC → memref.alloc
                   memref.load          LOAD  → memref.load
                   memref.store         STORE → memref.store

R-Node (Region)  → memref.alloca       Scoped regions → alloca
                   custom dealloc pass  Deterministisches Freigeben
```

### Lowering-Pipeline

```
Phase 1: FLUX-Dialekt (eigener MLIR-Dialekt)
  → 1:1 Abbildung der FLUX-Nodes als MLIR-Ops
  → V-Nodes werden als Metadata annotiert (fuer Debug)
  → R-Nodes werden als Region-Attribute annotiert

Phase 2: FLUX → Standard-Dialekte
  → C-Nodes → arith/math
  → K-Nodes → scf (structured control flow)
  → M-Nodes → memref
  → R-Nodes → Scope-basierte alloc/dealloc Paare
  → E-Nodes → func.call + scf.if

Phase 3: Standard-Optimierungen
  → Canonicalization, CSE, LICM, Inlining
  → Affine-Loop-Optimierung (wenn anwendbar)
  → Vektorisierung (MLIR vector Dialekt)

Phase 4: Lowering zu LLVM-Dialekt
  → memref → llvm.alloca / llvm.call @malloc
  → scf → llvm.br / llvm.cond_br
  → arith → llvm.add / llvm.mul etc.

Phase 5: LLVM IR → Maschinencode
  → LLVM -O3 (Baseline)
  → Superoptimizer (Stufe 2+3, siehe Sek. 7)
  → Target: x86-64, AArch64, RISC-V, WASM
```


## 16. Scope und Grenzen

### Wofuer FLUX v3 geeignet ist

```
IDEAL (Contracts beweisbar, Superopt wirksam):
  - Algorithmische Kernels (Sortierung, Suche, Hashing)
  - Kryptographie-Primitives (AES, SHA, ChaCha20)
  - SIMD-Kernels (Bildverarbeitung, Audio-Processing)
  - Numerische Berechnungen (Lineare Algebra, FFT)
  - Datenstruktur-Operationen (B-Tree, Hash-Map)
  - Embedded/Bare-Metal (kleine, verifizierte Binaries)
  - Safety-Critical Code (formale Verifikation entscheidend)

GUT (Contracts teilweise beweisbar):
  - CLI-Tools (File-I/O, Text-Processing)
  - Systemdienste (Daemons, Services)
  - Netzwerk-Protokolle (Parser, State-Machines)
  - Spiele (Game-Logic, Rendering-Kernels)

SCHWIERIG (viele UNDECIDABLE Contracts, viel FFI):
  - GUI-Anwendungen (plattformspezifisch, Event-driven)
  - Datenbank-Engines (komplexe Concurrency, Durability)
  - Web-Server (dynamisch, viele externe Abhaengigkeiten)
  - ML-Inference (Floating-Point-Korrektheit schwer)

NICHT GEEIGNET (out of scope):
  - Betriebssystem-Kernel (braucht eigenes Memory-Modell)
  - Laufzeitsysteme (GC, Scheduler, JIT)
  - Dynamisch typisierte Programme
  - Programme die auf menschliche Lesbarkeit angewiesen sind
```

### Bekannte Grenzen

```
1. LLM-Faehigkeit:
   Heutige LLMs koennen FTL erzeugen (strukturierter Text),
   aber die Qualitaet fuer komplexe Graphen (>1000 Nodes) ist
   unbewiesen. Ein spezialisiertes Fine-Tuning ist noetig.
   → Risikominderung: MVP mit einfachen Graphen validieren

2. Prover-Skalierung:
   SMT-Solver haben superexponentielle Worst-Case-Komplexitaet.
   Fuer grosse Programme werden viele Contracts UNDECIDABLE sein.
   → Risikominderung: Gestaffelte Timeouts, Inkubation, LLM-Lean

3. Superoptimierung:
   Wirksam nur fuer kleine Funktionen (<30 Instruktionen).
   Grossteil des Codes faellt auf LLVM -O3 zurueck.
   → Erwartung kalibriert: 5-20% Gesamtverbesserung, nicht 40%

4. Graph-Komplexitaet:
   Flacher Graph ohne Partitionierung skaliert nicht auf
   100.000+ Nodes. Inkrementelle Verifikation fehlt.
   → Zukuenftiges Feature: Graph-Partitionierung (nicht P-Node!)

5. Concurrency:
   Nur Fork-Join und atomare Operationen. Kein Runtime-Scheduler.
   → Fuer komplexe Concurrency: FLUX erzeugt Kernels,
     externes Runtime-System orchestriert

6. Kreativitaet:
   LLMs tendieren zu Mode Collapse. Die eigentliche kreative
   Kraft liegt im genetischen Algorithmus (GA), nicht im LLM.
   → Realistische Rollenverteilung: LLM = Initialisierung + Repair,
     GA = Innovation durch Mutation + Selektion

7. Training:
   Kein Trainingskorpus fuer FTL existiert. Henne-Ei-Problem.
   → Loesungspfad: Phase 1 synthetischer Korpus aus existierendem
     Code (C/Rust → FTL Transpiler), Phase 2 RLVF mit Validator
```

### MVP-Strategie

```
Das Minimum Viable Product validiert die riskanteste Annahme zuerst:
KANN ein LLM strukturell valide FTL-Graphen erzeugen?

Phase 1: FTL-Compiler + Validator (ohne Prover)
  → Eingabe: FTL-Text
  → Ausgabe: "VALID" oder strukturierte Fehlerliste
  → Test: LLM (GPT-4/Claude) erzeugt einfache FTL-Graphen
  → Metrik: Wie viele Graphen bestehen Validation auf Anhieb?

Phase 2: + Contract Prover (Z3 only, Phase 1)
  → Eingabe: Valider Graph + Contracts
  → Ausgabe: PROVEN / DISPROVEN mit Counterexample
  → Test: LLM reagiert auf Counterexamples und korrigiert
  → Metrik: Konvergiert der Feedback-Loop?

Phase 3: + MLIR Lowering + LLVM Compilation
  → Eingabe: Bewiesener Graph
  → Ausgabe: Nativer Maschinencode
  → Test: Hello World, Sortierung, Fibonacci
  → Metrik: Korrektheit und Groesse des Binary

Phase 4: + Superoptimierung (Stufe 2+3)
  → Test: Vergleich mit LLVM -O3 only
  → Metrik: Instruktionsanzahl, Zyklen

Phase 5: + Pool / Evolution / Explorative Synthese
  → Test: OPTIMIERE/ERFINDE auf Sortier-Algorithmen
  → Metrik: Findet das System genuein bessere Varianten?
```
