//! Emit G002's frozen memory-backed versus complete-local-GPR interpreter model.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};
use wasmparser::{Operator, Parser, Payload, Validator};

const GPRS: u32 = 32;
const WRITABLE_GPRS: u32 = 31;
const RECORDS: u32 = 1_024;
const RECORD_BASE: i32 = 4_096;
const PARAMS: u32 = 3;
const I32_SCRATCH: u32 = 6;
const I64_SCRATCH: u32 = 4;

const M4: MemArg = MemArg {
    offset: 0,
    align: 2,
    memory_index: 0,
};
const M8: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};

#[derive(Clone, Copy, Debug)]
enum Storage {
    Memory,
    Local,
}

impl Storage {
    fn name(self) -> &'static str {
        match self {
            Self::Memory => "control",
            Self::Local => "treatment",
        }
    }

    fn state_locals(self) -> u32 {
        match self {
            Self::Memory => 0,
            Self::Local => WRITABLE_GPRS,
        }
    }
}

#[derive(Clone, Copy)]
struct Locals {
    record: u32,
    index: u32,
    remaining: u32,
    round: u32,
    register_index: u32,
    shift: u32,
    a: u32,
    b: u32,
    output: u32,
    checksum: u32,
}

fn locals(storage: Storage) -> Locals {
    let i32_base = PARAMS + storage.state_locals();
    let i64_base = i32_base + I32_SCRATCH;
    Locals {
        record: i32_base,
        index: i32_base + 1,
        remaining: i32_base + 2,
        round: i32_base + 3,
        register_index: i32_base + 4,
        shift: i32_base + 5,
        a: i64_base,
        b: i64_base + 1,
        output: i64_base + 2,
        checksum: i64_base + 3,
    }
}

fn state_local(register: u32) -> u32 {
    assert!((1..GPRS).contains(&register));
    PARAMS + register - 1
}

fn state_address(function: &mut Function, register_index: u32) {
    function.instruction(&Instruction::LocalGet(register_index));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
}

/// Leave the dynamically selected state value on the operand stack.
fn local_state_get(function: &mut Function, register_index: u32) {
    // One result block encloses 32 empty case blocks. Target N exits case
    // block N and lands immediately before the body for xN.
    function.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
    for _ in 0..GPRS {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(register_index));
    function.instruction(&Instruction::BrTable(
        (0..GPRS).collect::<Vec<_>>().into(),
        0,
    ));
    for register in 0..GPRS {
        function.instruction(&Instruction::End);
        if register == 0 {
            function.instruction(&Instruction::I64Const(0));
        } else {
            function.instruction(&Instruction::LocalGet(state_local(register)));
        }
        function.instruction(&Instruction::Br(GPRS - register - 1));
    }
    function.instruction(&Instruction::End);
}

/// Store OUTPUT in the dynamically selected state value, discarding x0.
fn local_state_set(function: &mut Function, register_index: u32, output: u32) {
    function.instruction(&Instruction::Block(BlockType::Empty));
    for _ in 0..GPRS {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(register_index));
    function.instruction(&Instruction::BrTable(
        (0..GPRS).collect::<Vec<_>>().into(),
        0,
    ));
    for register in 0..GPRS {
        function.instruction(&Instruction::End);
        if register != 0 {
            function.instruction(&Instruction::LocalGet(output));
            function.instruction(&Instruction::LocalSet(state_local(register)));
        }
        function.instruction(&Instruction::Br(GPRS - register - 1));
    }
    function.instruction(&Instruction::End);
}

fn dynamic_state_get(function: &mut Function, storage: Storage, register_index: u32) {
    match storage {
        Storage::Memory => {
            state_address(function, register_index);
            function.instruction(&Instruction::I64Load(M8));
        }
        Storage::Local => local_state_get(function, register_index),
    }
}

fn dynamic_state_set(function: &mut Function, storage: Storage, register_index: u32, output: u32) {
    match storage {
        Storage::Memory => {
            // The architecture discards writes to x0.
            function.instruction(&Instruction::LocalGet(register_index));
            function.instruction(&Instruction::I32Eqz);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::Else);
            state_address(function, register_index);
            function.instruction(&Instruction::LocalGet(output));
            function.instruction(&Instruction::I64Store(M8));
            function.instruction(&Instruction::End);
        }
        Storage::Local => local_state_set(function, register_index, output),
    }
}

fn load_local_state(function: &mut Function) {
    for register in 1..GPRS {
        function.instruction(&Instruction::I32Const((register * 8) as i32));
        function.instruction(&Instruction::I64Load(M8));
        function.instruction(&Instruction::LocalSet(state_local(register)));
    }
}

fn commit_local_state(function: &mut Function) {
    for register in 1..GPRS {
        function.instruction(&Instruction::I32Const((register * 8) as i32));
        function.instruction(&Instruction::LocalGet(state_local(register)));
        function.instruction(&Instruction::I64Store(M8));
    }
}

fn emit_register_index(function: &mut Function, record: u32, shift: i32, output: u32) {
    function.instruction(&Instruction::LocalGet(record));
    if shift != 0 {
        function.instruction(&Instruction::I32Const(shift));
        function.instruction(&Instruction::I32ShrU);
    }
    function.instruction(&Instruction::I32Const(31));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(output));
}

fn driver(storage: Storage) -> Function {
    let locals = locals(storage);
    let mut local_groups = Vec::new();
    if storage.state_locals() != 0 {
        local_groups.push((storage.state_locals(), ValType::I64));
    }
    local_groups.push((I32_SCRATCH, ValType::I32));
    local_groups.push((I64_SCRATCH, ValType::I64));
    let mut function = Function::new(local_groups);

    if matches!(storage, Storage::Local) {
        load_local_state(&mut function);
    }
    function.instruction(&Instruction::I64Const(0x243f_6a88_85a3_08d3u64 as i64));
    function.instruction(&Instruction::LocalSet(locals.checksum));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(locals.round));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(locals.round));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::BrIf(1));

    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::LocalSet(locals.index));
    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::LocalSet(locals.remaining));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(locals.remaining));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));

    function.instruction(&Instruction::I32Const(RECORD_BASE));
    function.instruction(&Instruction::LocalGet(locals.index));
    function.instruction(&Instruction::I32Const(2));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Load(M4));
    function.instruction(&Instruction::LocalSet(locals.record));

    emit_register_index(&mut function, locals.record, 0, locals.register_index);
    dynamic_state_get(&mut function, storage, locals.register_index);
    function.instruction(&Instruction::LocalSet(locals.a));

    emit_register_index(&mut function, locals.record, 5, locals.register_index);
    dynamic_state_get(&mut function, storage, locals.register_index);
    function.instruction(&Instruction::LocalSet(locals.b));

    function.instruction(&Instruction::LocalGet(locals.record));
    function.instruction(&Instruction::I32Const(15));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::I32Const(63));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(locals.shift));

    function.instruction(&Instruction::LocalGet(locals.a));
    function.instruction(&Instruction::LocalGet(locals.record));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalGet(locals.b));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(locals.shift));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Rotl);
    function.instruction(&Instruction::LocalSet(locals.output));

    emit_register_index(&mut function, locals.record, 10, locals.register_index);
    dynamic_state_set(&mut function, storage, locals.register_index, locals.output);

    function.instruction(&Instruction::LocalGet(locals.checksum));
    function.instruction(&Instruction::LocalGet(locals.output));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::LocalGet(locals.record));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(7));
    function.instruction(&Instruction::I64Rotl);
    function.instruction(&Instruction::LocalSet(locals.checksum));

    function.instruction(&Instruction::LocalGet(locals.index));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(locals.index));
    function.instruction(&Instruction::LocalGet(locals.remaining));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(locals.remaining));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(locals.round));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(locals.round));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    if matches!(storage, Storage::Local) {
        commit_local_state(&mut function);
    }

    // Both variants compute the final digest from committed linear memory.
    for register in 0..GPRS {
        function.instruction(&Instruction::LocalGet(locals.checksum));
        function.instruction(&Instruction::I64Const(5));
        function.instruction(&Instruction::I64Rotl);
        function.instruction(&Instruction::I32Const((register * 8) as i32));
        function.instruction(&Instruction::I64Load(M8));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::LocalSet(locals.checksum));
    }
    function.instruction(&Instruction::LocalGet(locals.checksum));
    function.instruction(&Instruction::End);
    function
}

fn initial_state(register: u32) -> u64 {
    if register == 0 {
        return 0;
    }
    let mut value = 0x243f_6a88_85a3_08d3u64
        .wrapping_add(0x9e37_79b9_7f4a_7c15u64.wrapping_mul(u64::from(register)));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31) ^ u64::from(register)
}

fn reset_function() -> Function {
    let mut function = Function::new([]);
    for register in 0..GPRS {
        function.instruction(&Instruction::I32Const((register * 8) as i32));
        function.instruction(&Instruction::I64Const(initial_state(register) as i64));
        function.instruction(&Instruction::I64Store(M8));
    }
    function.instruction(&Instruction::End);
    function
}

fn state_word_function() -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32Const(31));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I64Load(M8));
    function.instruction(&Instruction::End);
    function
}

fn record_function() -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::I32Const(RECORD_BASE));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32Const(2));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Load(M4));
    function.instruction(&Instruction::End);
    function
}

fn constant_i32_function(value: i32) -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::I32Const(value));
    function.instruction(&Instruction::End);
    function
}

fn records() -> Vec<u32> {
    (0..RECORDS)
        .map(|index| {
            let lo = index & 31;
            let hi = index >> 5;
            let rd = lo;
            let rs1 = (hi + 5 * lo + 1) & 31;
            let rs2 = (7 * hi + 13 * lo + 3) & 31;
            let shift = ((17 * hi + 11 * lo) % 63) + 1;
            let salt = (0x5a3 * hi + 0x31d * lo + 0x155) & 0x7ff;
            rs1 | (rs2 << 5) | (rd << 10) | (shift << 15) | (salt << 21)
        })
        .collect()
}

fn module(records: &[u32]) -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I32, ValType::I32], [ValType::I64]);
    types.ty().function([], []);
    types.ty().function([ValType::I32], [ValType::I64]);
    types.ty().function([ValType::I32], [ValType::I32]);
    types.ty().function([], [ValType::I32]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    functions.function(0); // run_control
    functions.function(0); // run_treatment
    functions.function(1); // reset_state
    functions.function(2); // state_word
    functions.function(3); // record_at
    functions.function(4); // record_count
    functions.function(4); // state_word_count
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
    exports.export("run_control", ExportKind::Func, 0);
    exports.export("run_treatment", ExportKind::Func, 1);
    exports.export("reset_state", ExportKind::Func, 2);
    exports.export("state_word", ExportKind::Func, 3);
    exports.export("record_at", ExportKind::Func, 4);
    exports.export("record_count", ExportKind::Func, 5);
    exports.export("state_word_count", ExportKind::Func, 6);
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&driver(Storage::Memory));
    code.function(&driver(Storage::Local));
    code.function(&reset_function());
    code.function(&state_word_function());
    code.function(&record_function());
    code.function(&constant_i32_function(RECORDS as i32));
    code.function(&constant_i32_function(GPRS as i32));
    module.section(&code);

    let record_bytes = records
        .iter()
        .flat_map(|record| record.to_le_bytes())
        .collect::<Vec<_>>();
    let mut data = DataSection::new();
    data.active(
        0,
        &ConstExpr::i32_const(RECORD_BASE),
        record_bytes.iter().copied(),
    );
    module.section(&data);
    module.finish()
}

#[derive(Default)]
struct FunctionShape {
    i32_locals: u32,
    i64_locals: u32,
    operators: u32,
    i32_loads: u32,
    i64_loads: u32,
    i64_stores: u32,
    br_tables: Vec<u32>,
    calls: u32,
}

fn inspect(bytes: &[u8]) -> Result<Vec<FunctionShape>, String> {
    Validator::new()
        .validate_all(bytes)
        .map_err(|error| format!("validation failed: {error}"))?;
    let mut shapes = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        if let Payload::CodeSectionEntry(body) =
            payload.map_err(|error| format!("parse failed: {error}"))?
        {
            let mut shape = FunctionShape::default();
            let mut locals = body
                .get_locals_reader()
                .map_err(|error| format!("locals failed: {error}"))?;
            for _ in 0..locals.get_count() {
                let (count, ty) = locals
                    .read()
                    .map_err(|error| format!("local failed: {error}"))?;
                match ty {
                    wasmparser::ValType::I32 => shape.i32_locals += count,
                    wasmparser::ValType::I64 => shape.i64_locals += count,
                    _ => return Err(format!("unexpected local type {ty:?}")),
                }
            }
            let mut operators = body
                .get_operators_reader()
                .map_err(|error| format!("operators failed: {error}"))?;
            while !operators.eof() {
                shape.operators += 1;
                match operators
                    .read()
                    .map_err(|error| format!("operator failed: {error}"))?
                {
                    Operator::I32Load { .. } => shape.i32_loads += 1,
                    Operator::I64Load { .. } => shape.i64_loads += 1,
                    Operator::I64Store { .. } => shape.i64_stores += 1,
                    Operator::BrTable { targets } => {
                        shape.br_tables.push(targets.len() + 1);
                    }
                    Operator::Call { .. } | Operator::CallIndirect { .. } => shape.calls += 1,
                    _ => {}
                }
            }
            shapes.push(shape);
        }
    }
    Ok(shapes)
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: g002_local_gpr_model OUTPUT_DIRECTORY");
    fs::create_dir_all(&output).expect("create output directory");

    let records = records();
    let bytes = module(&records);
    let shapes = inspect(&bytes).expect("inspect G002 model");
    assert_eq!(shapes.len(), 7);
    fs::write(output.join("model.wasm"), &bytes).expect("write G002 model");

    let mut record_bytes = Vec::with_capacity(records.len() * 4);
    for record in &records {
        record_bytes.extend_from_slice(&record.to_le_bytes());
    }
    fs::write(output.join("records.bin"), record_bytes).expect("write G002 records");

    let mut schedule = String::from("index\trd\trs1\trs2\tshift\tsalt\tpacked\n");
    for (index, record) in records.iter().copied().enumerate() {
        writeln!(
            schedule,
            "{index}\t{}\t{}\t{}\t{}\t{}\t0x{record:08x}",
            (record >> 10) & 31,
            record & 31,
            (record >> 5) & 31,
            (record >> 15) & 63,
            record >> 21,
        )
        .expect("write schedule");
    }
    fs::write(output.join("schedule.tsv"), schedule).expect("write G002 schedule");

    let mut shape = String::from(
        "function\ti32_locals\ti64_locals\toperators\ti32_loads\ti64_loads\ti64_stores\tbr_tables\tcalls\n",
    );
    let names = [
        Storage::Memory.name(),
        Storage::Local.name(),
        "reset_state",
        "state_word",
        "record_at",
        "record_count",
        "state_word_count",
    ];
    for (name, function) in names.into_iter().zip(&shapes) {
        writeln!(
            shape,
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            function.i32_locals,
            function.i64_locals,
            function.operators,
            function.i32_loads,
            function.i64_loads,
            function.i64_stores,
            function
                .br_tables
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            function.calls,
        )
        .expect("write shape");
    }
    fs::write(output.join("shape.tsv"), shape).expect("write G002 shape");
}
