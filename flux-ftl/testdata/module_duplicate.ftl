// Duplicate node ID test — defines T:shared_int which is also in module_a.ftl
import "module_a.ftl"
T:shared_int = integer { bits: 32, signed: false }
