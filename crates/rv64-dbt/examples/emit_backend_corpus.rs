//! Emit deterministic standalone modules used by the backend lifecycle and
//! execution benchmark. The two modules implement identical six-member RV64
//! indirect-control cycles; only the architectural-state strategy differs.

use rv64_dbt::{translate_superblock_sparse_state, JitLayout, MultiEntryState, SystemMemory};
use std::env;
use std::fs;
use std::path::PathBuf;

const LOAD_TAGS: u32 = 512;
const LOAD_OFFSETS: u32 = LOAD_TAGS + 4096 * 8;
const STORE_TAGS: u32 = LOAD_OFFSETS + 4096 * 8;
const STORE_OFFSETS: u32 = STORE_TAGS + 4096 * 8;
const ACCESS_CONTEXT: u32 = STORE_OFFSETS + 4096 * 8;

fn enc_i(op: u32, funct3: u32, rd: u32, rs1: u32, imm: i32) -> u32 {
    op | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (((imm as u32) & 0x0fff) << 20)
}

fn enc_u(op: u32, rd: u32, imm20: i32) -> u32 {
    op | (rd << 7) | (((imm20 as u32) & 0x000f_ffff) << 12)
}

fn enc_s(op: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = (imm as u32) & 0x0fff;
    op | ((imm & 0x1f) << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | ((imm >> 5) << 25)
}

fn corpus_code() -> Vec<u8> {
    let mut words = Vec::new();
    for block in 0..6 {
        let target_delta = if block == 5 { -0x54 } else { 0x0c };
        words.extend([
            enc_i(0x13, 0, 1, 1, block + 1), // addi x1,x1,increment
            enc_u(0x17, 10, 0),              // auipc x10,0
            enc_i(0x13, 0, 10, 10, target_delta),
            enc_i(0x67, 0, 0, 10, 0), // jalr x0,0(x10)
        ]);
    }
    let mut code: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    code.resize(4096, 0);
    code
}

fn memory_corpus_code() -> Vec<u8> {
    let mut words = Vec::new();
    for block in 0..6 {
        let target_delta = if block == 5 { -0x1d4 } else { 0x0c };
        for register in 2..10 {
            words.push(enc_i(0x03, 3, register, 20, (register as i32 - 2) * 8));
        }
        for register in 2..10 {
            words.push(enc_s(0x23, 3, 20, register, 64 + (register as i32 - 2) * 8));
        }
        words.extend([
            enc_i(0x13, 0, 1, 1, 1), // addi x1,x1,1
            enc_u(0x17, 10, 0),      // auipc x10,0
            enc_i(0x13, 0, 10, 10, target_delta),
            enc_i(0x67, 0, 0, 10, 0), // jalr x0,0(x10)
        ]);
    }
    let mut code: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    code.resize(4096, 0);
    code
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-backend-corpus"));
    fs::create_dir_all(&output).expect("create corpus directory");

    let code = corpus_code();
    let entries: Vec<u64> = (0..6).map(|block| 0x1000 + block * 0x10).collect();
    let mut layout = JitLayout::bare();
    // x0..x31 = 0..255, PC = 256, retired = 264, fuel = 272.
    layout.fuel_addr = 272;

    for (name, state) in [
        ("cached", MultiEntryState::RegisterEager),
        ("lazy", MultiEntryState::RegisterLazy),
        ("direct", MultiEntryState::RegisterDirect),
        ("materialized", MultiEntryState::Memory),
        ("tailcall", MultiEntryState::MemoryTailCall),
    ] {
        let region = translate_superblock_sparse_state(&code, &[0x1000], &entries, layout, state)
            .expect("translate benchmark region");
        fs::write(output.join(format!("{name}.wasm")), region.wasm)
            .expect("write generated module");
    }

    let memory_code = memory_corpus_code();
    let memory_entries: Vec<u64> = (0..6).map(|block| 0x1000 + block * 0x50).collect();
    for (name, state, cache_translations) in [
        ("cached-memory", MultiEntryState::RegisterEager, true),
        (
            "cached-memory-no-tlb",
            MultiEntryState::RegisterEager,
            false,
        ),
        ("lazy-memory", MultiEntryState::RegisterLazy, false),
        ("direct-memory", MultiEntryState::RegisterDirect, false),
        ("materialized-memory", MultiEntryState::Memory, false),
        ("tailcall-memory", MultiEntryState::MemoryTailCall, false),
    ] {
        let mut memory_layout = layout;
        memory_layout.sys = Some(
            SystemMemory::fused_4k(
                LOAD_TAGS,
                LOAD_OFFSETS,
                STORE_TAGS,
                STORE_OFFSETS,
                ACCESS_CONTEXT,
                4095,
                false,
            )
            .with_invocation_cache(cache_translations),
        );
        let region = translate_superblock_sparse_state(
            &memory_code,
            &[0x1000],
            &memory_entries,
            memory_layout,
            state,
        )
        .expect("translate memory benchmark region");
        fs::write(output.join(format!("{name}.wasm")), region.wasm)
            .expect("write generated memory module");
    }
}
