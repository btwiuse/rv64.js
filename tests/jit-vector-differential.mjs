// Exercise every mandatory RVV encoding from the QEMU differential at one
// stable hot PC, then compare the complete guest-produced state record between
// the authoritative interpreter and generated Wasm execution.
process.env.RVV_JIT_DIFFERENTIAL = "1";
process.env.RVV_SKIP_QEMU = "1";
process.env.RVV_CASE_REPETITIONS ||= "128";
await import("./rvv-interpreter-differential.mjs");
