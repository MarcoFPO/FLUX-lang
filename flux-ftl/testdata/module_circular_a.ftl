// Circular import test — A imports B
import "module_circular_b.ftl"
T:circ_a = integer { bits: 32, signed: false }
