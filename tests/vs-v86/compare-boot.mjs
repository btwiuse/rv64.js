#!/usr/bin/env node

// Kept as the focused boot-comparison entry point. The implementation lives
// in matched-boot.mjs so this command and the full scorecard use the same
// Linux, Alpine, memory, initramfs, firmware, and readiness contract.
await import("./matched-boot.mjs");
