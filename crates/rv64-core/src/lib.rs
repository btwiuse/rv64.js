//! rv64-core: portable RV64 CPU core.
//!
//! Design (see /DESIGN.md): the CPU is generic over a [`Bus`], so the same
//! execute code serves both user-mode emulation (flat memory + syscall shim)
//! and full-system emulation (MMU walk + MMIO dispatch). No I/O, no alloc
//! beyond the caller-provided memory — the wasm crate and native harnesses
//! own the outside world.

#![cfg_attr(not(test), no_std)]

pub mod bus;
pub mod compressed;
pub mod cpu;
pub mod csr;
pub mod decode;
pub mod exception;

pub use bus::{Bus, FlatMemory};
pub use cpu::{Cpu, StopReason};
pub use exception::Exception;
