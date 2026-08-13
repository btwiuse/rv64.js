//! Emit R117's frozen local-state and module-global-state pressure models.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};
use wasmparser::{Operator, Parser, Payload, Validator};

const STATE: u32 = 31;
const STATE_LOCAL_BASE: u32 = 2;
const M8: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};

#[derive(Clone, Copy)]
enum Storage {
    Local,
    Global,
}

#[derive(Clone, Copy)]
struct LocalIndices {
    acc: u32,
    temp: u32,
    index: u32,
}

impl Storage {
    fn locals(self) -> LocalIndices {
        let base = match self {
            Storage::Local => STATE_LOCAL_BASE + STATE,
            Storage::Global => STATE_LOCAL_BASE,
        };
        LocalIndices {
            acc: base,
            temp: base + 1,
            index: base + 2,
        }
    }
}

fn get(function: &mut Function, storage: Storage, index: u32) {
    function.instruction(&match storage {
        Storage::Local => Instruction::LocalGet(STATE_LOCAL_BASE + index),
        Storage::Global => Instruction::GlobalGet(index),
    });
}

fn set(function: &mut Function, storage: Storage, index: u32) {
    function.instruction(&match storage {
        Storage::Local => Instruction::LocalSet(STATE_LOCAL_BASE + index),
        Storage::Global => Instruction::GlobalSet(index),
    });
}

fn emit(storage: Storage) -> Vec<u8> {
    let locals = storage.locals();
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I64]);
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
    if matches!(storage, Storage::Global) {
        let mut globals = GlobalSection::new();
        for index in 0..STATE {
            globals.global(
                GlobalType {
                    val_type: ValType::I64,
                    mutable: true,
                    shared: false,
                },
                &wasm_encoder::ConstExpr::i64_const(index as i64 + 1),
            );
        }
        module.section(&globals);
    }
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("run", ExportKind::Func, 0);
    module.section(&exports);

    let mut local_groups = Vec::new();
    if matches!(storage, Storage::Local) {
        local_groups.push((STATE, ValType::I64));
    }
    local_groups.extend([(2, ValType::I64), (1, ValType::I32)]);
    let mut function = Function::new(local_groups);

    // Every call initializes the complete architectural state from memory.
    for index in 0..STATE {
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I32Const((index * 8) as i32));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::I64Load(M8));
        set(&mut function, storage, index);
    }
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(locals.acc));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(locals.index));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(locals.index));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::BrIf(1));

    // First phase touches every value while preserving them all for the
    // separated second phase.
    for index in 0..STATE {
        get(&mut function, storage, index);
        get(&mut function, storage, (index + 7) % STATE);
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const(
            0x9e37_79b9u32.wrapping_mul(index + 1) as i32 as i64,
        ));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalGet(locals.index));
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::I64Xor);
        set(&mut function, storage, index);
    }

    // Structured conditional work models branch-adjacent state pressure.
    get(&mut function, storage, 3);
    get(&mut function, storage, 19);
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    get(&mut function, storage, 5);
    get(&mut function, storage, 23);
    function.instruction(&Instruction::I64Add);
    set(&mut function, storage, 11);
    function.instruction(&Instruction::Else);
    get(&mut function, storage, 11);
    function.instruction(&Instruction::I64Const(17));
    function.instruction(&Instruction::I64Rotl);
    set(&mut function, storage, 11);
    function.instruction(&Instruction::End);

    // Second separated phase consumes every retained value and updates a
    // checksum. TEMP keeps the operation shape identical between modules.
    for index in (0..STATE).rev() {
        get(&mut function, storage, index);
        get(&mut function, storage, (index + 13) % STATE);
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalTee(locals.temp));
        function.instruction(&Instruction::I64Const(((index % 63) + 1) as i64));
        function.instruction(&Instruction::I64Rotr);
        set(&mut function, storage, index);
        function.instruction(&Instruction::LocalGet(locals.acc));
        function.instruction(&Instruction::LocalGet(locals.temp));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::LocalSet(locals.acc));
    }
    function.instruction(&Instruction::LocalGet(locals.index));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(locals.index));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    for index in 0..STATE {
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I32Const((index * 8) as i32));
        function.instruction(&Instruction::I32Add);
        get(&mut function, storage, index);
        function.instruction(&Instruction::I64Store(M8));
    }
    function.instruction(&Instruction::LocalGet(locals.acc));
    function.instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&function);
    module.section(&code);
    module.finish()
}

#[derive(Default)]
struct Shape {
    bytes: usize,
    functions: u32,
    mutable_i64_globals: u32,
    local_i64: u32,
    local_i32: u32,
    operators: u32,
    state_get: [u32; STATE as usize],
    state_set: [u32; STATE as usize],
    nonstate_local_get: u32,
    nonstate_local_set: u32,
    nonstate_local_tee: u32,
    memory_load: u32,
    memory_store: u32,
}

fn local_name(storage: Storage, index: u32) -> Option<&'static str> {
    let locals = storage.locals();
    if index == 0 {
        Some("param.memory_base")
    } else if index == 1 {
        Some("param.iterations")
    } else if index == locals.acc {
        Some("local.acc")
    } else if index == locals.temp {
        Some("local.temp")
    } else if index == locals.index {
        Some("local.index")
    } else {
        None
    }
}

fn normalize_operator(
    operator: &Operator<'_>,
    storage: Storage,
    shape: &mut Shape,
) -> Result<String, String> {
    shape.operators += 1;
    let normalized = match *operator {
        Operator::LocalGet { local_index }
            if matches!(storage, Storage::Local)
                && (STATE_LOCAL_BASE..STATE_LOCAL_BASE + STATE).contains(&local_index) =>
        {
            let state_index = (local_index - STATE_LOCAL_BASE) as usize;
            shape.state_get[state_index] += 1;
            format!("state.get:{state_index}")
        }
        Operator::LocalSet { local_index }
            if matches!(storage, Storage::Local)
                && (STATE_LOCAL_BASE..STATE_LOCAL_BASE + STATE).contains(&local_index) =>
        {
            let state_index = (local_index - STATE_LOCAL_BASE) as usize;
            shape.state_set[state_index] += 1;
            format!("state.set:{state_index}")
        }
        Operator::GlobalGet { global_index } if matches!(storage, Storage::Global) => {
            let state_index = usize::try_from(global_index)
                .map_err(|_| format!("global index {global_index} does not fit usize"))?;
            if state_index >= STATE as usize {
                return Err(format!("unexpected global.get {global_index}"));
            }
            shape.state_get[state_index] += 1;
            format!("state.get:{state_index}")
        }
        Operator::GlobalSet { global_index } if matches!(storage, Storage::Global) => {
            let state_index = usize::try_from(global_index)
                .map_err(|_| format!("global index {global_index} does not fit usize"))?;
            if state_index >= STATE as usize {
                return Err(format!("unexpected global.set {global_index}"));
            }
            shape.state_set[state_index] += 1;
            format!("state.set:{state_index}")
        }
        Operator::LocalGet { local_index } => {
            shape.nonstate_local_get += 1;
            format!(
                "local.get:{}",
                local_name(storage, local_index)
                    .ok_or_else(|| format!("unexpected local.get {local_index}"))?
            )
        }
        Operator::LocalSet { local_index } => {
            shape.nonstate_local_set += 1;
            format!(
                "local.set:{}",
                local_name(storage, local_index)
                    .ok_or_else(|| format!("unexpected local.set {local_index}"))?
            )
        }
        Operator::LocalTee { local_index } => {
            shape.nonstate_local_tee += 1;
            format!(
                "local.tee:{}",
                local_name(storage, local_index)
                    .ok_or_else(|| format!("unexpected local.tee {local_index}"))?
            )
        }
        Operator::I64Load { .. } => {
            shape.memory_load += 1;
            format!("{operator:?}")
        }
        Operator::I64Store { .. } => {
            shape.memory_store += 1;
            format!("{operator:?}")
        }
        Operator::GlobalGet { global_index } | Operator::GlobalSet { global_index } => {
            return Err(format!("unexpected global operator {global_index}"));
        }
        _ => format!("{operator:?}"),
    };
    Ok(normalized)
}

fn inspect(bytes: &[u8], storage: Storage) -> Result<(Shape, String), String> {
    Validator::new()
        .validate_all(bytes)
        .map_err(|error| format!("validation failed: {error}"))?;
    let mut shape = Shape {
        bytes: bytes.len(),
        ..Shape::default()
    };
    let mut normalized = String::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|error| format!("parse failed: {error}"))? {
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global.map_err(|error| format!("global parse failed: {error}"))?;
                    if global.ty.mutable && global.ty.content_type == wasmparser::ValType::I64 {
                        shape.mutable_i64_globals += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                shape.functions += 1;
                let mut locals = body
                    .get_locals_reader()
                    .map_err(|error| format!("locals parse failed: {error}"))?;
                for _ in 0..locals.get_count() {
                    let (count, ty) = locals
                        .read()
                        .map_err(|error| format!("local parse failed: {error}"))?;
                    match ty {
                        wasmparser::ValType::I64 => shape.local_i64 += count,
                        wasmparser::ValType::I32 => shape.local_i32 += count,
                        _ => return Err(format!("unexpected local type {ty:?}")),
                    }
                }
                let mut operators = body
                    .get_operators_reader()
                    .map_err(|error| format!("operators parse failed: {error}"))?;
                while !operators.eof() {
                    let operator = operators
                        .read()
                        .map_err(|error| format!("operator parse failed: {error}"))?;
                    writeln!(
                        normalized,
                        "{}",
                        normalize_operator(&operator, storage, &mut shape)?
                    )
                    .expect("write normalized operator");
                }
            }
            _ => {}
        }
    }
    Ok((shape, normalized))
}

fn append_shape(report: &mut String, side: &str, shape: &Shape) {
    writeln!(
        report,
        "{side}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        shape.bytes,
        shape.functions,
        shape.mutable_i64_globals,
        shape.local_i64,
        shape.local_i32,
        shape.operators,
        shape.state_get.iter().sum::<u32>(),
        shape.state_set.iter().sum::<u32>(),
        shape.nonstate_local_get + shape.nonstate_local_set + shape.nonstate_local_tee,
        shape.memory_load,
        shape.memory_store,
    )
    .expect("write shape row");
    for index in 0..STATE as usize {
        writeln!(
            report,
            "{side}.state.{index}\t{}\t{}",
            shape.state_get[index], shape.state_set[index]
        )
        .expect("write state row");
    }
}

fn write_model(output: &Path, side: &str, bytes: &[u8], normalized: &str) {
    fs::write(output.join(format!("{side}.wasm")), bytes).expect("write model");
    fs::write(output.join(format!("{side}.normalized.txt")), normalized)
        .expect("write normalized operators");
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/bench/r117-module-global/model"));
    fs::create_dir_all(&output).expect("create output directory");
    let local = emit(Storage::Local);
    let global = emit(Storage::Global);
    let (local_shape, local_normalized) = inspect(&local, Storage::Local).expect("inspect local");
    let (global_shape, global_normalized) =
        inspect(&global, Storage::Global).expect("inspect global");
    assert_eq!(
        local_normalized, global_normalized,
        "normalized operator streams differ"
    );
    assert_eq!(local_shape.state_get, global_shape.state_get);
    assert_eq!(local_shape.state_set, global_shape.state_set);
    write_model(&output, "local", &local, &local_normalized);
    write_model(&output, "global", &global, &global_normalized);
    let mut report = String::from(
        "side\tbytes\tfunctions\tmutable_i64_globals\tlocal_i64\tlocal_i32\toperators\tstate_get\tstate_set\tnonstate_local_ops\tmemory_load\tmemory_store\n",
    );
    append_shape(&mut report, "local", &local_shape);
    append_shape(&mut report, "global", &global_shape);
    fs::write(output.join("shape.tsv"), report).expect("write shape report");
}
