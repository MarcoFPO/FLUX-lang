// Minimal valid FTL graph
// 1 T-Node, 1 C-Node, 1 K-Node, entry

T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
