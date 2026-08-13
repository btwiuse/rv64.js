//! Emit tiny linking modules for executing the frozen backend corpus with a
//! standalone Wasm CLI. `env.wasm` owns the imported linear memory;
//! `driver-{integer,memory}.wasm` initialize it, invoke `jit.run`, and trap if
//! any architectural result differs from the benchmark contract.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection, ImportSection,
    Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

const PC_ADDR: u64 = 256;
const RETIRED_ADDR: u64 = 264;
const FUEL_ADDR: u64 = 272;
const ENTRY_PC: i64 = 0x1000;
const FUEL: i64 = 216_000;
const LOAD_TAGS: u64 = 512;
const LOAD_OFFSETS: u64 = LOAD_TAGS + 4096 * 8;
const STORE_TAGS: u64 = LOAD_OFFSETS + 4096 * 8;
const STORE_OFFSETS: u64 = STORE_TAGS + 4096 * 8;
const ACCESS_CONTEXT: u64 = STORE_OFFSETS + 4096 * 8;
const ROW_OFFSET: u64 = 2 * 8;
const HOST_DATA: u64 = 0x24000;
const TRANSLATION_OFFSET: i64 = HOST_DATA as i64 - 0x2000;

const fn memarg(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
        memory_index: 0,
    }
}

fn store(function: &mut Function, address: u64, value: i64) {
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I64Const(value));
    function.instruction(&Instruction::I64Store(memarg(address)));
}

fn expect(function: &mut Function, address: u64, value: i64) {
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I64Load(memarg(address)));
    function.instruction(&Instruction::I64Const(value));
    function.instruction(&Instruction::I64Ne);
    function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
}

fn environment_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 3,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);
    module.finish()
}

fn driver_module(memory_cycle: bool) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], []);
    types.ty().function([], []);
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        },
    );
    imports.import("jit", "run", EntityType::Function(0));
    module.section(&imports);

    let mut functions = FunctionSection::new();
    functions.function(1);
    module.section(&functions);
    let mut exports = ExportSection::new();
    // Function index 0 is the imported jit.run.
    exports.export("check", ExportKind::Func, 1);
    module.section(&exports);

    let mut function = Function::new([]);
    store(&mut function, PC_ADDR, ENTRY_PC);
    store(&mut function, RETIRED_ADDR, 0);
    store(&mut function, FUEL_ADDR, FUEL);
    if memory_cycle {
        store(&mut function, 20 * 8, 0x2000);
        // Fused TLB tags carry the complete guest page base ORed with the
        // current access-context tag, not the virtual page number.  The test
        // context is zero, so VA 0x2000 is represented by tag 0x2000.
        store(&mut function, LOAD_TAGS + ROW_OFFSET, 0x2000);
        store(&mut function, LOAD_OFFSETS + ROW_OFFSET, TRANSLATION_OFFSET);
        store(&mut function, STORE_TAGS + ROW_OFFSET, 0x2000);
        store(
            &mut function,
            STORE_OFFSETS + ROW_OFFSET,
            TRANSLATION_OFFSET,
        );
        store(&mut function, ACCESS_CONTEXT, 0);
        for index in 0..8 {
            store(&mut function, HOST_DATA + index * 8, 0x1000 + index as i64);
            store(&mut function, HOST_DATA + 64 + index * 8, 0);
        }
    }
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::Call(0));
    expect(&mut function, PC_ADDR, ENTRY_PC);
    expect(&mut function, RETIRED_ADDR, FUEL);
    if memory_cycle {
        expect(&mut function, 8, (FUEL / 120) * 6);
        for index in 0..8 {
            expect(
                &mut function,
                HOST_DATA + 64 + index * 8,
                0x1000 + index as i64,
            );
        }
    } else {
        expect(&mut function, 8, (FUEL / 24) * 21);
    }
    function.instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&function);
    module.section(&code);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-standalone-gate"));
    fs::create_dir_all(&output).expect("create standalone gate directory");
    fs::write(output.join("env.wasm"), environment_module()).expect("write env module");
    fs::write(output.join("driver-integer.wasm"), driver_module(false))
        .expect("write integer driver");
    fs::write(output.join("driver-memory.wasm"), driver_module(true)).expect("write memory driver");
}
