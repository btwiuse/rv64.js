//! Emit R124's frozen eager-state and RV64C-bank hybrid-state pressure models.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
    MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};
use wasmparser::{Operator, Parser, Payload, Validator};

const FIRST_X: u32 = 1;
const LAST_X: u32 = 31;
const STATE_COUNT: u32 = LAST_X - FIRST_X + 1;
const RVC_BANK_MASK: u32 = 0x0000_ff06;
const PARAM_COUNT: u32 = 2;
const SCRATCH_COUNT: u32 = 5;
const M8: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};

#[derive(Clone, Copy)]
enum Storage {
    Eager,
    Hybrid,
}

#[derive(Clone, Copy)]
struct Locals {
    acc: u32,
    lhs: u32,
    rhs: u32,
    third: u32,
    output: u32,
    iteration: u32,
}

fn resident(reg: u32) -> bool {
    RVC_BANK_MASK & (1u32 << reg) != 0
}

fn resident_position(reg: u32) -> Option<u32> {
    resident(reg).then(|| {
        (FIRST_X..reg)
            .filter(|candidate| resident(*candidate))
            .count() as u32
    })
}

impl Storage {
    fn state_local_count(self) -> u32 {
        match self {
            Self::Eager => STATE_COUNT,
            Self::Hybrid => RVC_BANK_MASK.count_ones(),
        }
    }

    fn state_local(self, reg: u32) -> Option<u32> {
        assert!((FIRST_X..=LAST_X).contains(&reg));
        match self {
            Self::Eager => Some(PARAM_COUNT + reg - FIRST_X),
            Self::Hybrid => resident_position(reg).map(|position| PARAM_COUNT + position),
        }
    }

    fn locals(self) -> Locals {
        let base = PARAM_COUNT + self.state_local_count();
        Locals {
            acc: base,
            lhs: base + 1,
            rhs: base + 2,
            third: base + 3,
            output: base + 4,
            iteration: base + SCRATCH_COUNT,
        }
    }
}

fn state_offset(reg: u32) -> u64 {
    u64::from((reg - FIRST_X) * 8)
}

fn read_state(function: &mut Function, storage: Storage, reg: u32) {
    if let Some(local) = storage.state_local(reg) {
        function.instruction(&Instruction::LocalGet(local));
    } else {
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Load(MemArg {
            offset: state_offset(reg),
            ..M8
        }));
    }
}

fn write_output(function: &mut Function, storage: Storage, reg: u32, output: u32) {
    if let Some(local) = storage.state_local(reg) {
        function.instruction(&Instruction::LocalGet(output));
        function.instruction(&Instruction::LocalSet(local));
    } else {
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::LocalGet(output));
        function.instruction(&Instruction::I64Store(MemArg {
            offset: state_offset(reg),
            ..M8
        }));
    }
}

fn source(reg: u32, displacement: u32) -> u32 {
    FIRST_X + (reg - FIRST_X + displacement) % STATE_COUNT
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
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("run", ExportKind::Func, 0);
    module.section(&exports);

    let mut function = Function::new([
        (storage.state_local_count() + SCRATCH_COUNT, ValType::I64),
        (1, ValType::I32),
    ]);
    for reg in FIRST_X..=LAST_X {
        if let Some(local) = storage.state_local(reg) {
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I64Load(MemArg {
                offset: state_offset(reg),
                ..M8
            }));
            function.instruction(&Instruction::LocalSet(local));
        }
    }
    function.instruction(&Instruction::I64Const(0x243f_6a88_85a3_08d3u64 as i64));
    function.instruction(&Instruction::LocalSet(locals.acc));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalSet(locals.iteration));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(locals.iteration));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::BrIf(1));

    // One deterministic architecture-wide round updates every writable GPR.
    // Rotated sources give every register the same role without using a guest
    // trace, compiler output, or measured popularity.
    for reg in FIRST_X..=LAST_X {
        let rhs = source(reg, 7);
        let third = source(reg, 13);
        read_state(&mut function, storage, reg);
        function.instruction(&Instruction::LocalSet(locals.lhs));
        read_state(&mut function, storage, rhs);
        function.instruction(&Instruction::LocalSet(locals.rhs));
        read_state(&mut function, storage, third);
        function.instruction(&Instruction::LocalSet(locals.third));

        function.instruction(&Instruction::LocalGet(locals.lhs));
        function.instruction(&Instruction::LocalGet(locals.rhs));
        function.instruction(&Instruction::I64LtU);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        function.instruction(&Instruction::LocalGet(locals.lhs));
        function.instruction(&Instruction::LocalGet(locals.rhs));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalGet(locals.third));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const(
            0x9e37_79b9u32.wrapping_mul(reg) as i32 as i64,
        ));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalGet(locals.iteration));
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const(i64::from((reg % 63) + 1)));
        function.instruction(&Instruction::I64Rotl);
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::LocalGet(locals.lhs));
        function.instruction(&Instruction::LocalGet(locals.third));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::LocalGet(locals.rhs));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::I64Const(
            0x85eb_ca6bu32.wrapping_mul(reg) as i32 as i64,
        ));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const(i64::from((reg * 3 % 63) + 1)));
        function.instruction(&Instruction::I64Rotr);
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalSet(locals.output));

        write_output(&mut function, storage, reg, locals.output);
        function.instruction(&Instruction::LocalGet(locals.acc));
        function.instruction(&Instruction::LocalGet(locals.output));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const(i64::from((reg * 5 % 63) + 1)));
        function.instruction(&Instruction::I64Rotl);
        function.instruction(&Instruction::LocalSet(locals.acc));
    }

    function.instruction(&Instruction::LocalGet(locals.iteration));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(locals.iteration));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    // Eager state commits its complete dirty union. Hybrid nonresident state
    // is already canonical after every member and commits only the fixed bank.
    for reg in FIRST_X..=LAST_X {
        if let Some(local) = storage.state_local(reg) {
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::LocalGet(local));
            function.instruction(&Instruction::I64Store(MemArg {
                offset: state_offset(reg),
                ..M8
            }));
        }
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
    local_i64: u32,
    local_i32: u32,
    operators: u32,
    local_get: u32,
    local_set: u32,
    memory_load: u32,
    memory_store: u32,
    branches: u32,
}

fn inspect(bytes: &[u8]) -> Result<Shape, String> {
    Validator::new()
        .validate_all(bytes)
        .map_err(|error| format!("validation failed: {error}"))?;
    let mut shape = Shape {
        bytes: bytes.len(),
        ..Shape::default()
    };
    for payload in Parser::new(0).parse_all(bytes) {
        if let Payload::CodeSectionEntry(body) =
            payload.map_err(|error| format!("parse failed: {error}"))?
        {
            shape.functions += 1;
            let mut locals = body
                .get_locals_reader()
                .map_err(|error| format!("locals failed: {error}"))?;
            for _ in 0..locals.get_count() {
                let (count, ty) = locals
                    .read()
                    .map_err(|error| format!("local failed: {error}"))?;
                match ty {
                    wasmparser::ValType::I64 => shape.local_i64 += count,
                    wasmparser::ValType::I32 => shape.local_i32 += count,
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
                    Operator::LocalGet { .. } => shape.local_get += 1,
                    Operator::LocalSet { .. } => shape.local_set += 1,
                    Operator::I64Load { .. } => shape.memory_load += 1,
                    Operator::I64Store { .. } => shape.memory_store += 1,
                    Operator::If { .. }
                    | Operator::Else
                    | Operator::Br { .. }
                    | Operator::BrIf { .. } => shape.branches += 1,
                    _ => {}
                }
            }
        }
    }
    Ok(shape)
}

fn write_shape(report: &mut String, name: &str, shape: &Shape) {
    writeln!(
        report,
        "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        shape.bytes,
        shape.functions,
        shape.local_i64,
        shape.local_i32,
        shape.operators,
        shape.local_get,
        shape.local_set,
        shape.memory_load,
        shape.memory_store,
        shape.branches,
    )
    .expect("write shape");
}

fn write_artifact(output: &Path, name: &str, bytes: &[u8]) {
    fs::write(output.join(format!("{name}.wasm")), bytes).expect("write model");
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/bench/r124-rvc-bank-hybrid/model"));
    fs::create_dir_all(&output).expect("create output directory");
    let eager = emit(Storage::Eager);
    let hybrid = emit(Storage::Hybrid);
    let eager_shape = inspect(&eager).expect("inspect eager");
    let hybrid_shape = inspect(&hybrid).expect("inspect hybrid");
    write_artifact(&output, "eager", &eager);
    write_artifact(&output, "hybrid", &hybrid);

    let mut shape = String::from(
        "variant\tbytes\tfunctions\tlocal_i64\tlocal_i32\toperators\tlocal_get\tlocal_set\tmemory_load\tmemory_store\tbranches\n",
    );
    write_shape(&mut shape, "eager", &eager_shape);
    write_shape(&mut shape, "hybrid", &hybrid_shape);
    fs::write(output.join("shape.tsv"), shape).expect("write shape");

    let mut schedule = String::from("member\tdestination\trhs\tthird\tresident_destination\n");
    for reg in FIRST_X..=LAST_X {
        writeln!(
            schedule,
            "{}\tx{}\tx{}\tx{}\t{}",
            reg,
            reg,
            source(reg, 7),
            source(reg, 13),
            resident(reg) as u8,
        )
        .expect("write schedule");
    }
    fs::write(output.join("schedule.tsv"), schedule).expect("write schedule");
}
