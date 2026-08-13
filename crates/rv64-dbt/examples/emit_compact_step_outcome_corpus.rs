//! Emit the frozen R058 compact step-outcome engine-shape corpus.
//!
//! The control materializes the accepted Rust structure-return layout in
//! linear memory. The candidate returns one i32 and writes exception sidecar
//! state only on the rare error path. Both drivers retain one direct call to a
//! deliberately non-inlineable (>500 wire-byte) step function.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
    MemArg, MemorySection, MemoryType, Module, NameMap, NameSection, TypeSection, ValType,
};

const STATE: i32 = 0;
const RESULT: i32 = 64;
const PADDING_NOPS: usize = 640;
const INITIAL_ACC: i64 = 0x0123_4567_89ab_cdef;
const STEP_BIAS: i64 = 0x61c8_8646_80b5_83eb;
const ERROR_KIND: i64 = 13;

const I64_AT_0: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};
const I64_AT_8: MemArg = MemArg {
    offset: 8,
    align: 3,
    memory_index: 0,
};
const I64_AT_16: MemArg = MemArg {
    offset: 16,
    align: 3,
    memory_index: 0,
};
const I64_AT_24: MemArg = MemArg {
    offset: 24,
    align: 3,
    memory_index: 0,
};
const I32_AT_8: MemArg = MemArg {
    offset: 8,
    align: 2,
    memory_index: 0,
};

#[derive(Clone, Copy)]
enum Variant {
    Sret,
    Compact,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Sret => "sret",
            Self::Compact => "compact",
        }
    }

    fn step_parameters(self) -> u32 {
        match self {
            Self::Sret => 3,
            Self::Compact => 2,
        }
    }

    fn state_parameter(self) -> u32 {
        match self {
            Self::Sret => 1,
            Self::Compact => 0,
        }
    }

    fn mode_parameter(self) -> u32 {
        match self {
            Self::Sret => 2,
            Self::Compact => 1,
        }
    }
}

fn emit_i64_store(function: &mut Function, address: i32, value: i64, arg: MemArg) {
    function.instruction(&Instruction::I32Const(address));
    function.instruction(&Instruction::I64Const(value));
    function.instruction(&Instruction::I64Store(arg));
}

fn step(variant: Variant) -> Function {
    let parameter_count = variant.step_parameters();
    let acc = parameter_count;
    let counter = parameter_count + 1;
    let outcome = parameter_count + 2;
    let state = variant.state_parameter();
    let mode = variant.mode_parameter();
    let mut function = Function::new([(2, ValType::I64), (1, ValType::I32)]);

    // Semantically inert but part of both frozen bytes. V8 14.6's declared
    // maximum Wasm inlinee size is 500 wire bytes; this keeps the direct call
    // while making the executed body an intentionally generous ABI upper bound.
    for _ in 0..PADDING_NOPS {
        function.instruction(&Instruction::Nop);
    }

    function.instruction(&Instruction::LocalGet(state));
    function.instruction(&Instruction::I64Load(I64_AT_0));
    function.instruction(&Instruction::LocalSet(acc));
    function.instruction(&Instruction::LocalGet(state));
    function.instruction(&Instruction::I64Load(I64_AT_8));
    function.instruction(&Instruction::LocalSet(counter));

    function.instruction(&Instruction::LocalGet(acc));
    function.instruction(&Instruction::LocalGet(counter));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::I64Const(13));
    function.instruction(&Instruction::I64Rotl);
    function.instruction(&Instruction::I64Const(STEP_BIAS));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(acc));
    function.instruction(&Instruction::LocalGet(state));
    function.instruction(&Instruction::LocalGet(acc));
    function.instruction(&Instruction::I64Store(I64_AT_0));

    function.instruction(&Instruction::LocalGet(counter));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(counter));
    function.instruction(&Instruction::LocalGet(state));
    function.instruction(&Instruction::LocalGet(counter));
    function.instruction(&Instruction::I64Store(I64_AT_8));

    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(outcome));
    function.instruction(&Instruction::LocalGet(mode));
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(mode));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::I32Const(1)); // clean stop
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I32Const(4)); // exception
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalSet(outcome));
    function.instruction(&Instruction::End);

    match variant {
        Variant::Sret => {
            function.instruction(&Instruction::LocalGet(outcome));
            function.instruction(&Instruction::I32Const(4));
            function.instruction(&Instruction::I32Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I64Const(1));
            function.instruction(&Instruction::I64Store(I64_AT_0));
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I32Const(ERROR_KIND as i32));
            function.instruction(&Instruction::I32Store(I32_AT_8));
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::LocalGet(counter));
            function.instruction(&Instruction::I64Store(I64_AT_16));
            function.instruction(&Instruction::Else);
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::I64Store(I64_AT_0));
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I32Const(-1));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::LocalGet(outcome));
            function.instruction(&Instruction::I32Eqz);
            function.instruction(&Instruction::Select);
            function.instruction(&Instruction::I32Store(I32_AT_8));
            function.instruction(&Instruction::End);
        }
        Variant::Compact => {
            function.instruction(&Instruction::LocalGet(outcome));
            function.instruction(&Instruction::I32Const(4));
            function.instruction(&Instruction::I32Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::LocalGet(state));
            function.instruction(&Instruction::I64Const(ERROR_KIND));
            function.instruction(&Instruction::I64Store(I64_AT_16));
            function.instruction(&Instruction::LocalGet(state));
            function.instruction(&Instruction::LocalGet(counter));
            function.instruction(&Instruction::I64Store(I64_AT_24));
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::LocalGet(outcome));
        }
    }

    function.instruction(&Instruction::End);
    function
}

// Driver parameters: iterations i32, mode i32. Locals: remaining i32,
// outcome i32, checksum i64.
const REMAINING: u32 = 2;
const OUTCOME: u32 = 3;
const CHECKSUM: u32 = 4;

fn driver(variant: Variant) -> Function {
    let mut function = Function::new([(2, ValType::I32), (1, ValType::I64)]);
    emit_i64_store(&mut function, STATE, INITIAL_ACC, I64_AT_0);
    emit_i64_store(&mut function, STATE, 0, I64_AT_8);
    emit_i64_store(&mut function, STATE, 0, I64_AT_16);
    emit_i64_store(&mut function, STATE, 0, I64_AT_24);
    emit_i64_store(&mut function, RESULT, 0, I64_AT_0);
    emit_i64_store(&mut function, RESULT, 0, I64_AT_8);
    emit_i64_store(&mut function, RESULT, 0, I64_AT_16);
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(OUTCOME));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    match variant {
        Variant::Sret => {
            function.instruction(&Instruction::I32Const(RESULT));
            function.instruction(&Instruction::I32Const(STATE));
            function.instruction(&Instruction::LocalGet(1));
            function.instruction(&Instruction::Call(0));

            function.instruction(&Instruction::I32Const(RESULT));
            function.instruction(&Instruction::I64Load(I64_AT_0));
            function.instruction(&Instruction::I64Eqz);
            function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            function.instruction(&Instruction::I32Const(RESULT));
            function.instruction(&Instruction::I32Load(I32_AT_8));
            function.instruction(&Instruction::I32Const(-1));
            function.instruction(&Instruction::I32Eq);
            function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::Else);
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::Else);
            function.instruction(&Instruction::I32Const(STATE));
            function.instruction(&Instruction::I32Const(RESULT));
            function.instruction(&Instruction::I32Load(I32_AT_8));
            function.instruction(&Instruction::I64ExtendI32U);
            function.instruction(&Instruction::I64Store(I64_AT_16));
            function.instruction(&Instruction::I32Const(STATE));
            function.instruction(&Instruction::I32Const(RESULT));
            function.instruction(&Instruction::I64Load(I64_AT_16));
            function.instruction(&Instruction::I64Store(I64_AT_24));
            function.instruction(&Instruction::I32Const(4));
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::LocalSet(OUTCOME));
        }
        Variant::Compact => {
            function.instruction(&Instruction::I32Const(STATE));
            function.instruction(&Instruction::LocalGet(1));
            function.instruction(&Instruction::Call(0));
            function.instruction(&Instruction::LocalSet(OUTCOME));
        }
    }

    function.instruction(&Instruction::LocalGet(OUTCOME));
    function.instruction(&Instruction::BrIf(1));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::I32Const(STATE));
    function.instruction(&Instruction::I64Load(I64_AT_0));
    function.instruction(&Instruction::I32Const(STATE));
    function.instruction(&Instruction::I64Load(I64_AT_8));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::I32Const(STATE));
    function.instruction(&Instruction::I64Load(I64_AT_16));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::I32Const(STATE));
    function.instruction(&Instruction::I64Load(I64_AT_24));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(OUTCOME));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalSet(CHECKSUM));

    // Normalize non-architectural result scratch so complete memory snapshots
    // compare byte-for-byte after all normal, stop, and exception probes.
    emit_i64_store(&mut function, RESULT, 0, I64_AT_0);
    emit_i64_store(&mut function, RESULT, 0, I64_AT_8);
    emit_i64_store(&mut function, RESULT, 0, I64_AT_16);
    function.instruction(&Instruction::LocalGet(CHECKSUM));
    function.instruction(&Instruction::End);
    function
}

fn module(variant: Variant) -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    match variant {
        Variant::Sret => types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I32], []),
        Variant::Compact => types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]),
    };
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I64]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0);
    functions.function(1);
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
    exports.export("run", ExportKind::Func, 1);
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&step(variant));
    code.function(&driver(variant));
    module.section(&code);

    let mut names = NameSection::new();
    names.module(&format!("r058-{}", variant.name()));
    let mut function_names = NameMap::new();
    function_names.append(0, &format!("{}_step", variant.name()));
    function_names.append(1, &format!("{}_driver", variant.name()));
    names.functions(&function_names);
    module.section(&names);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: emit_compact_step_outcome_corpus OUTPUT_DIRECTORY");
    fs::create_dir_all(&output).expect("create output directory");
    for variant in [Variant::Sret, Variant::Compact] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant),
        )
        .expect("write corpus module");
    }
}
