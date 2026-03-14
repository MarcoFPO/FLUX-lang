// Circular import test — B imports A
import "module_circular_a.ftl"
T:circ_b = integer { bits: 32, signed: false }
