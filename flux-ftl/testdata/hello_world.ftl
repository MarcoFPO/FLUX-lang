// Hello World — FLUX v3 FTL
// Spec Section 11: Minimal-Beispiel

// Typen
T:a1 = array { element: u8, max_length: 12 }
T:a2 = integer { bits: 64, signed: false }
T:a3 = unit

// Regionen
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
