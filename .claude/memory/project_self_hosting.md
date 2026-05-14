---
name: Self-Hosting Ziel
description: FLUX soll sich selbst kompilieren können — "Flux createt on Flux", kein Rust/Cargo mehr nötig
type: project
---

Das übergeordnete Ziel des FLUX-Projekts ist Self-Hosting: Der FLUX-Compiler soll in FTL selbst geschrieben werden, sodass Rust/Cargo nicht mehr benötigt wird.

**Why:** FLUX soll eine eigenständige Sprache sein, die sich selbst ausdrücken kann. Solange der Compiler in Rust geschrieben ist, bleibt eine externe Abhängigkeit bestehen.

**How to apply:** Alle zukünftigen Phasen und Spracherweiterungen sollten darauf ausgerichtet sein, FTL ausdrucksstark genug zu machen, um den eigenen Compiler zu schreiben. Priorität haben Features, die für einen Compiler nötig sind (Strings, Rekursion, dynamische Datenstrukturen, File I/O, LLVM-FFI).
