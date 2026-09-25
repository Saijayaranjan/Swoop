// SwiftPM needs one C source file for the target that vends the UniFFI header + module map.
// The implementation lives in the Rust static library `libswoop_ffi.a`.
void swoop_ffi_swiftpm_anchor(void) {}
