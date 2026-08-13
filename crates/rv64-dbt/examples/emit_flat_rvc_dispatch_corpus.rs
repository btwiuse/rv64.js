//! Emit the frozen R059 nested-versus-flat RV64C dispatch corpus.
//!
//! This is an engine-shape upper bound, not an RV64 implementation. Both
//! variants execute all 24 quadrant/funct3 families and identical handler
//! work. The control uses one outer plus three inner `br_table`s; the candidate
//! uses one combined-selector `br_table`.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, Instruction, MemArg, MemorySection, MemoryType, Module, NameMap, NameSection,
    TypeSection, ValType,
};

const FAMILIES: u32 = 24;
const STREAM_BYTES: u32 = 4096;
const INITIAL_STATE: i64 = 0x0123_4567_89ab_cdef;

#[derive(Clone, Copy)]
enum Variant {
    Nested,
    Flat,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Nested => "nested",
            Self::Flat => "flat",
        }
    }
}

// Parameter 0 is operation count. Locals: stream index, remaining, combined
// selector, and architectural state.
const INDEX: u32 = 1;
const REMAINING: u32 = 2;
const SELECTOR: u32 = 3;
const STATE: u32 = 4;

fn emit_handler(function: &mut Function, family: u32) {
    // Distinct immediates prevent case merging. Keeping executed handler work
    // deliberately small makes this a favorable upper bound for dispatch.
    let immediate = ((u64::from(family) + 1) * 0x9e37_79b9) | 1;
    function.instruction(&Instruction::LocalGet(STATE));
    function.instruction(&Instruction::I64Const(immediate as i64));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalSet(STATE));
}

fn emit_flat_dispatch(function: &mut Function) {
    function.instruction(&Instruction::Block(BlockType::Empty)); // dispatch exit
    for _ in 0..FAMILIES {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(SELECTOR));
    function.instruction(&Instruction::BrTable(
        (0..FAMILIES).collect::<Vec<_>>().into(),
        FAMILIES,
    ));
    for family in 0..FAMILIES {
        function.instruction(&Instruction::End);
        emit_handler(function, family);
        function.instruction(&Instruction::Br(FAMILIES - family - 1));
    }
    function.instruction(&Instruction::End);
}

fn emit_inner_dispatch(function: &mut Function, quadrant: u32) {
    let outer_remaining = 2 - quadrant;
    for _ in 0..8 {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(SELECTOR));
    function.instruction(&Instruction::I32Const(7));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::BrTable(
        (0..8).collect::<Vec<_>>().into(),
        8 + outer_remaining,
    ));
    for funct3 in 0..8 {
        function.instruction(&Instruction::End);
        emit_handler(function, quadrant * 8 + funct3);
        function.instruction(&Instruction::Br(7 - funct3 + outer_remaining));
    }
}

fn emit_nested_dispatch(function: &mut Function) {
    function.instruction(&Instruction::Block(BlockType::Empty)); // dispatch exit
    for _ in 0..3 {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(SELECTOR));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::BrTable(vec![0, 1, 2].into(), 3));
    for quadrant in 0..3 {
        function.instruction(&Instruction::End);
        emit_inner_dispatch(function, quadrant);
    }
    function.instruction(&Instruction::End);
}

fn driver(variant: Variant) -> Function {
    let mut function = Function::new([(3, ValType::I32), (1, ValType::I64)]);
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(INDEX));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::I64Const(INITIAL_STATE));
    function.instruction(&Instruction::LocalSet(STATE));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    function.instruction(&Instruction::LocalGet(INDEX));
    function.instruction(&Instruction::I32Const((STREAM_BYTES - 1) as i32));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Load8U(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
    function.instruction(&Instruction::LocalSet(SELECTOR));
    match variant {
        Variant::Nested => emit_nested_dispatch(&mut function),
        Variant::Flat => emit_flat_dispatch(&mut function),
    }

    function.instruction(&Instruction::LocalGet(INDEX));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(INDEX));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(STATE));
    function.instruction(&Instruction::LocalGet(INDEX));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::End);
    function
}

fn selector_stream() -> Vec<u8> {
    (0..STREAM_BYTES)
        .map(|index| ((index * 7 + 5) % FAMILIES) as u8)
        .collect()
}

fn module(variant: Variant, stream: &[u8]) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], [ValType::I64]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

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
    exports.export("run", ExportKind::Func, 0);
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&driver(variant));
    module.section(&code);

    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(0), stream.iter().copied());
    module.section(&data);

    let mut names = NameSection::new();
    names.module(&format!("r059-{}", variant.name()));
    let mut function_names = NameMap::new();
    function_names.append(0, &format!("{}_driver", variant.name()));
    names.functions(&function_names);
    module.section(&names);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: emit_flat_rvc_dispatch_corpus OUTPUT_DIRECTORY");
    fs::create_dir_all(&output).expect("create output directory");
    let stream = selector_stream();
    for variant in [Variant::Nested, Variant::Flat] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant, &stream),
        )
        .expect("write corpus module");
    }
}
