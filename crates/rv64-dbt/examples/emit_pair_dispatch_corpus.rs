//! Emit the frozen R053 pair-dispatch engine-shape corpus.
//!
//! This is deliberately not an RV64 implementation. It isolates the backend
//! shape that a precompiled, architecture-complete two-instruction tier would
//! add: one indirect handler dispatch per scalar instruction versus one
//! exhaustive specialized handler for every ordered pair. Both modules run
//! the same operation stream and produce identical architectural state.

use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, Elements, ExportKind,
    ExportSection, Function, FunctionSection, Instruction, MemArg, MemorySection, MemoryType,
    Module, RefType, TableSection, TableType, TypeSection, ValType,
};

const OPERATION_KINDS: u32 = 62;
const PAIR_KINDS: u32 = OPERATION_KINDS * OPERATION_KINDS;
const STREAM_PAIRS: u32 = 4096;
const SINGLE_STREAM_BYTES: u32 = STREAM_PAIRS * 2;

#[derive(Clone, Copy)]
enum Variant {
    Single,
    Pair,
}

impl Variant {
    fn handler_count(self) -> u32 {
        match self {
            Self::Single => OPERATION_KINDS,
            Self::Pair => PAIR_KINDS,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Pair => "pair",
        }
    }
}

fn emit_operation(function: &mut Function, operation: u32) {
    let target = operation % 4;
    let source = (target + 1 + operation % 2) % 4;
    let other = (target + 2 + (operation / 2) % 2) % 4;
    let kind = operation / 4;
    let immediate = ((operation as i64 + 1) * 0x9e37) | 1;

    match kind {
        0 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Add);
        }
        1 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Sub);
        }
        2 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Xor);
        }
        3 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Or);
        }
        4 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64And);
        }
        5 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Mul);
        }
        6 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Shl);
        }
        7 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64ShrU);
        }
        8 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64ShrS);
        }
        9 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Rotl);
        }
        10 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Rotr);
        }
        11 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::LocalGet(other));
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::I64LtU);
            function.instruction(&Instruction::Select);
        }
        12 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::I64Const(immediate));
            function.instruction(&Instruction::I64Add);
        }
        13 => {
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::I64Const(immediate));
            function.instruction(&Instruction::I64Xor);
        }
        14 => {
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I64Extend32S);
        }
        15 => {
            // Only operation IDs 60 and 61 reach this partial family.
            function.instruction(&Instruction::LocalGet(target));
            function.instruction(&Instruction::I64Popcnt);
        }
        _ => unreachable!("62 operations fit the 16 fixed lowering families"),
    }
    function.instruction(&Instruction::LocalSet(target));
}

fn handler(variant: Variant, index: u32) -> Function {
    let mut function = Function::new([]);
    match variant {
        Variant::Single => emit_operation(&mut function, index),
        Variant::Pair => {
            emit_operation(&mut function, index / OPERATION_KINDS);
            emit_operation(&mut function, index % OPERATION_KINDS);
        }
    }
    for local in 0..4 {
        function.instruction(&Instruction::LocalGet(local));
    }
    function.instruction(&Instruction::End);
    function
}

fn driver(variant: Variant) -> Function {
    // Parameter 0 is the requested scalar-operation count. Locals 1..4 are
    // architectural i64 state; locals 5 and 6 are stream index and remaining
    // handler count.
    let mut function = Function::new([(4, ValType::I64), (2, ValType::I32)]);
    for (local, value) in [
        (1, 0x0123_4567_89ab_cdef_u64 as i64),
        (2, 0xfedc_ba98_7654_3210_u64 as i64),
        (3, 0x0f1e_2d3c_4b5a_6978_u64 as i64),
        (4, 0x8877_6655_4433_2211_u64 as i64),
    ] {
        function.instruction(&Instruction::I64Const(value));
        function.instruction(&Instruction::LocalSet(local));
    }
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(5));
    function.instruction(&Instruction::LocalGet(0));
    if matches!(variant, Variant::Pair) {
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::I32ShrU);
    }
    function.instruction(&Instruction::LocalSet(6));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    for local in 1..=4 {
        function.instruction(&Instruction::LocalGet(local));
    }
    function.instruction(&Instruction::LocalGet(5));
    match variant {
        Variant::Single => {
            function.instruction(&Instruction::I32Const((SINGLE_STREAM_BYTES - 1) as i32));
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::I32Load8U(MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            }));
        }
        Variant::Pair => {
            function.instruction(&Instruction::I32Const((STREAM_PAIRS - 1) as i32));
            function.instruction(&Instruction::I32And);
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Shl);
            function.instruction(&Instruction::I32Load16U(MemArg {
                offset: 0,
                align: 1,
                memory_index: 0,
            }));
        }
    }
    function.instruction(&Instruction::CallIndirect {
        type_index: 0,
        table_index: 0,
    });
    for local in (1..=4).rev() {
        function.instruction(&Instruction::LocalSet(local));
    }

    function.instruction(&Instruction::LocalGet(5));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(5));
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(6));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(3));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(4));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::End);
    function
}

fn selector_streams() -> (Vec<u8>, Vec<u8>) {
    let mut single = Vec::with_capacity(SINGLE_STREAM_BYTES as usize);
    let mut pair = Vec::with_capacity((STREAM_PAIRS * 2) as usize);
    for stream_index in 0..STREAM_PAIRS {
        let pair_index = if stream_index < PAIR_KINDS {
            stream_index
        } else {
            // Fill the power-of-two tail with a deterministic traversal rather
            // than adding invalid table entries. Every ordered pair still
            // occurs at least once in each stream cycle.
            (stream_index * 37 + 17) % PAIR_KINDS
        };
        single.push((pair_index / OPERATION_KINDS) as u8);
        single.push((pair_index % OPERATION_KINDS) as u8);
        pair.extend_from_slice(&(pair_index as u16).to_le_bytes());
    }
    (single, pair)
}

fn module(variant: Variant, stream: Vec<u8>) -> Vec<u8> {
    let handler_count = variant.handler_count();
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function(
        [ValType::I64, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64, ValType::I64, ValType::I64, ValType::I64],
    );
    types.ty().function([ValType::I32], [ValType::I64]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    for _ in 0..handler_count {
        functions.function(0);
    }
    functions.function(1);
    module.section(&functions);

    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: handler_count as u64,
        maximum: Some(handler_count as u64),
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
    exports.export("run", ExportKind::Func, handler_count);
    // Exporting memory prevents the corpus from relying on an engine proving
    // the selector data immutable and folding the table index.
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);

    let function_indices: Vec<u32> = (0..handler_count).collect();
    let mut elements = ElementSection::new();
    elements.active(
        None,
        &ConstExpr::i32_const(0),
        Elements::Functions(Cow::Owned(function_indices)),
    );
    module.section(&elements);

    let mut code = CodeSection::new();
    for index in 0..handler_count {
        code.function(&handler(variant, index));
    }
    code.function(&driver(variant));
    module.section(&code);

    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(0), stream);
    module.section(&data);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-pair-dispatch-corpus"));
    fs::create_dir_all(&output).expect("create pair-dispatch corpus directory");
    let (single_stream, pair_stream) = selector_streams();
    for (variant, stream) in [
        (Variant::Single, single_stream),
        (Variant::Pair, pair_stream),
    ] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant, stream),
        )
        .expect("write pair-dispatch corpus module");
    }
}
