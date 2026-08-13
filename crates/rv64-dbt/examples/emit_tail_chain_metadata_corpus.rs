//! Emit R097's proof-only tail-chain dispatch-metadata corpus.
//!
//! Both functions execute the complete generated tail-transfer predicate. The
//! control reloads `gen` and `idx` exactly like production; the candidate keeps
//! each first load in an otherwise dead i32 local. A cross-instance Wasm
//! barrier mutates the same memory between probes so the engine cannot hoist
//! proof loads across iterations.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, Instruction, MemArg, MemoryType, Module, TypeSection, ValType,
};

const DISPATCH_BASE: i32 = 0x1000;
const DISPATCH_MASK: i64 = 255;
const MAP_GEN_ADDR: i32 = 0x3000;
const BARRIER_ADDR: i32 = 0x3010;
const SB_INDEX_MASK: i32 = !(1 << 30);

const I32_MEM: MemArg = MemArg {
    offset: 0,
    align: 2,
    memory_index: 0,
};
const I64_MEM: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};

#[derive(Clone, Copy)]
enum Variant {
    Reload,
    Cached,
}

fn emit_probe(variant: Variant) -> Function {
    // params: 0=iterations:i32, 1=pc:i64
    // locals: 2=dispatch:i32, 3=checksum:i64, 4=gen:i32, 5=idx:i32
    // Both variants deliberately have identical local declarations.
    let mut function =
        Function::new_with_locals_types([ValType::I32, ValType::I64, ValType::I32, ValType::I32]);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(3));
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    // dispatch[((pc >> 1) & mask)] with the production 16-byte line layout.
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(DISPATCH_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const(4));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Const(DISPATCH_BASE));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(2));

    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I64Eq);

    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I32Load(MemArg {
        offset: 12,
        ..I32_MEM
    }));
    if matches!(variant, Variant::Cached) {
        function.instruction(&Instruction::LocalTee(4));
    }
    function.instruction(&Instruction::I32Const(MAP_GEN_ADDR));
    function.instruction(&Instruction::I32Load(I32_MEM));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::I32And);

    match variant {
        Variant::Reload => {
            function.instruction(&Instruction::LocalGet(2));
            function.instruction(&Instruction::I32Load(MemArg {
                offset: 12,
                ..I32_MEM
            }));
        }
        Variant::Cached => {
            function.instruction(&Instruction::LocalGet(4));
        }
    }
    function.instruction(&Instruction::I32Const(-1));
    function.instruction(&Instruction::I32Ne);
    function.instruction(&Instruction::I32And);

    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I32Load(MemArg {
        offset: 8,
        ..I32_MEM
    }));
    if matches!(variant, Variant::Cached) {
        function.instruction(&Instruction::LocalTee(5));
    }
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32GeS);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::If(BlockType::Empty));

    function.instruction(&Instruction::LocalGet(3));
    match variant {
        Variant::Reload => {
            function.instruction(&Instruction::LocalGet(2));
            function.instruction(&Instruction::I32Load(MemArg {
                offset: 8,
                ..I32_MEM
            }));
        }
        Variant::Cached => {
            function.instruction(&Instruction::LocalGet(5));
        }
    }
    function.instruction(&Instruction::I32Const(SB_INDEX_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(3));
    function.instruction(&Instruction::End);

    // Function import 0 is a Wasm-to-Wasm barrier that mutates this memory.
    function.instruction(&Instruction::Call(0));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(0));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(3));
    function.instruction(&Instruction::End);
    function
}

fn corpus_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);
    types
        .ty()
        .function([ValType::I32, ValType::I64], [ValType::I64]);
    module.section(&types);
    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: Some(1),
            memory64: false,
            shared: false,
            page_size_log2: None,
        }),
    );
    imports.import("env", "barrier", EntityType::Function(0));
    module.section(&imports);
    let mut functions = FunctionSection::new();
    functions.function(1);
    functions.function(1);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("reload", ExportKind::Func, 1);
    exports.export("cached", ExportKind::Func, 2);
    module.section(&exports);
    let mut code = CodeSection::new();
    code.function(&emit_probe(Variant::Reload));
    code.function(&emit_probe(Variant::Cached));
    module.section(&code);
    module.finish()
}

fn barrier_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);
    module.section(&types);
    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: Some(1),
            memory64: false,
            shared: false,
            page_size_log2: None,
        }),
    );
    module.section(&imports);
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("barrier", ExportKind::Func, 0);
    module.section(&exports);
    let mut function = Function::new([]);
    function.instruction(&Instruction::I32Const(BARRIER_ADDR));
    function.instruction(&Instruction::I32Const(BARRIER_ADDR));
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Store(I64_MEM));
    function.instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&function);
    module.section(&code);
    module.finish()
}

fn main() {
    let output = env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("usage: emit_tail_chain_metadata_corpus OUTPUT_DIR");
        std::process::exit(2);
    });
    fs::create_dir_all(&output).expect("create corpus directory");
    fs::write(output.join("corpus.wasm"), corpus_module()).expect("write corpus module");
    fs::write(output.join("barrier.wasm"), barrier_module()).expect("write barrier module");
}
