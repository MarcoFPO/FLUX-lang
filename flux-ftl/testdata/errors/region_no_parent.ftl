// Error test: scoped region without parent (6004)
// R:b2 is scoped but has no parent field — violates region hierarchy rules.

T:a1 = integer { bits: 32, signed: true }

// Static root — correct
R:b1 = region { lifetime: static }

// ERROR: scoped without parent
R:b2 = region { lifetime: scoped }

C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
