---
name: FTL muss selbstgenügsam sein
description: Jedes FTL-Programm muss ohne Codegen-Erweiterungen vollständig ausführbar sein — keine neuen Features pro Programm
type: feedback
---

FTL-Syntax muss grundlegend alle Programmkonstrukte abdecken, sodass bei jedem neuen Programm KEINE Erweiterung der Codegen/Compiler nötig ist.

**Why:** Der User hat explizit gesagt: "nicht bei jedem neuen Programm soll eine Erweiterung geben". FTL ist eine Sprache für LLMs — sie muss in sich geschlossen sein.

**How to apply:** Wenn ein FTL-Programm etwas nicht ausdrücken kann, ist die Lösung eine FTL-Spracherweiterung (Grammar + Parser + AST), NICHT ein Codegen-Hack oder hardcoded Sonderbehandlung.
