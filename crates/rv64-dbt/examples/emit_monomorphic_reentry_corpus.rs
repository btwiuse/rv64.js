//! Emit the frozen R056 interpreter re-entry callback corpus.
//!
//! Both variants execute the same direct step call and exact direct-mapped
//! dispatch-tag lookup. The control reaches the lookup through an externally
//! mutable Wasm table and `call_indirect`; the candidate spells it inline.
//! Memory and the table are exported so the engine cannot assume either proof
//! source is immutable.

use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ElementSection, Elements, ExportKind, ExportSection,
    Function, FunctionSection, Instruction, MemArg, MemorySection, MemoryType, Module, RefType,
    TableSection, TableType, TypeSection, ValType,
};

const DISPATCH_ENTRIES: i32 = 4096;
const DISPATCH_MASK: i32 = DISPATCH_ENTRIES - 1;
const DISPATCH_STRIDE_SHIFT: i32 = 4; // production DispatchLine is 16 bytes
const PC_BASE: i64 = 0x0000_5555_0000_0000;
const PC_LIMIT: i64 = PC_BASE + DISPATCH_ENTRIES as i64 * 2;
const INITIAL_STATE: i64 = 0x0123_4567_89ab_cdef;
const STEP_BIAS: i64 = 0x61c8_8646_80b5_83eb;
const HIT_BIAS: i64 = 0x2d35_8dcc_aa6c_78a5;

const TAG_LOAD: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};

#[derive(Clone, Copy)]
enum Variant {
    Indirect,
    Inline,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Indirect => "indirect",
            Self::Inline => "inline",
        }
    }
}

fn emit_tag_address(function: &mut Function, pc_local: u32) {
    function.instruction(&Instruction::LocalGet(pc_local));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const(DISPATCH_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Const(DISPATCH_STRIDE_SHIFT));
    function.instruction(&Instruction::I32Shl);
}

fn emit_exact_lookup(function: &mut Function, pc_local: u32) {
    emit_tag_address(function, pc_local);
    function.instruction(&Instruction::I64Load(TAG_LOAD));
    function.instruction(&Instruction::LocalGet(pc_local));
    function.instruction(&Instruction::I64Eq);
}

fn callback() -> Function {
    let mut function = Function::new([]);
    emit_exact_lookup(&mut function, 0);
    function.instruction(&Instruction::End);
    function
}

fn step() -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::I64Const(13));
    function.instruction(&Instruction::I64Rotl);
    function.instruction(&Instruction::I64Const(STEP_BIAS));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::End);
    function
}

// Driver locals after parameter 0: remaining i32, pc i64, state i64, hit i32.
const REMAINING: u32 = 1;
const PC: u32 = 2;
const STATE: u32 = 3;
const HIT: u32 = 4;

fn driver(variant: Variant) -> Function {
    let mut function = Function::new([(1, ValType::I32), (2, ValType::I64), (1, ValType::I32)]);
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::I64Const(PC_BASE));
    function.instruction(&Instruction::LocalSet(PC));
    function.instruction(&Instruction::I64Const(INITIAL_STATE));
    function.instruction(&Instruction::LocalSet(STATE));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    // Preserve the separate direct step call present in production. Its body
    // is identical in both variants and prevents this from becoming an empty
    // callback-only loop.
    function.instruction(&Instruction::LocalGet(STATE));
    function.instruction(&Instruction::LocalGet(PC));
    function.instruction(&Instruction::Call(1));
    function.instruction(&Instruction::LocalSet(STATE));

    function.instruction(&Instruction::LocalGet(PC));
    function.instruction(&Instruction::I64Const(2));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalTee(PC));
    function.instruction(&Instruction::I64Const(PC_LIMIT));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::I64Const(PC_BASE));
    function.instruction(&Instruction::LocalSet(PC));
    function.instruction(&Instruction::End);

    match variant {
        Variant::Indirect => {
            function.instruction(&Instruction::LocalGet(PC));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::CallIndirect {
                type_index: 0,
                table_index: 0,
            });
        }
        Variant::Inline => emit_exact_lookup(&mut function, PC),
    }
    function.instruction(&Instruction::LocalTee(HIT));
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(STATE));
    function.instruction(&Instruction::I64Const(HIT_BIAS));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalSet(STATE));
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(STATE));
    function.instruction(&Instruction::LocalGet(PC));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(HIT));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::End);
    function
}

fn module(variant: Variant) -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I32]);
    types
        .ty()
        .function([ValType::I64, ValType::I64], [ValType::I64]);
    types.ty().function([ValType::I32], [ValType::I64]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0);
    functions.function(1);
    functions.function(2);
    module.section(&functions);

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: 1,
        maximum: Some(1),
        shared: false,
    });
    module.section(&tables);

    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 2);
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("table", ExportKind::Table, 0);
    module.section(&exports);

    let mut elements = ElementSection::new();
    elements.active(
        None,
        &ConstExpr::i32_const(0),
        Elements::Functions(Cow::Owned(vec![0])),
    );
    module.section(&elements);

    let mut code = CodeSection::new();
    code.function(&callback());
    code.function(&step());
    code.function(&driver(variant));
    module.section(&code);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: emit_monomorphic_reentry_corpus OUTPUT_DIRECTORY");
    fs::create_dir_all(&output).expect("create output directory");
    for variant in [Variant::Indirect, Variant::Inline] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant),
        )
        .expect("write corpus module");
    }
}
