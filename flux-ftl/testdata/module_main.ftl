// Main module — imports module_a and uses its nodes
import "module_a.ftl"

E:exit = syscall_exit { inputs: [C:shared_zero], type: T:shared_int, effects: [PROC] }
K:main = seq { steps: [E:exit] }
entry: K:main
