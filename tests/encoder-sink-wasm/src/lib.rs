use std::hint::black_box;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

fn memarg(align: u32, offset: u64) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn enum_body(groups: u32) -> Vec<u8> {
    let mut function = Function::new_with_locals_types([ValType::I64, ValType::I32]);
    for group in 0..groups {
        let offset = u64::from((group & 31) * 8);
        function.instruction(&Instruction::Block(BlockType::Empty));
        function.instruction(&Instruction::Loop(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Const(i64::from(group)));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalTee(0));
        function.instruction(&Instruction::I32Const(4096));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Store(memarg(3, offset)));
        function.instruction(&Instruction::I32Const(4096));
        function.instruction(&Instruction::I64Load(memarg(3, offset)));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Eq);
        function.instruction(&Instruction::BrIf(1));
        function.instruction(&Instruction::LocalGet(1));
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::LocalSet(1));
        function.instruction(&Instruction::Br(0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::End);
    }
    function.instruction(&Instruction::End);
    function.into_raw_body()
}

fn sink_body(groups: u32) -> Vec<u8> {
    let mut function = Function::new_with_locals_types([ValType::I64, ValType::I32]);
    let mut sink = function.instructions();
    for group in 0..groups {
        let offset = u64::from((group & 31) * 8);
        sink.block(BlockType::Empty);
        sink.loop_(BlockType::Empty);
        sink.local_get(0);
        sink.i64_const(i64::from(group));
        sink.i64_add();
        sink.local_tee(0);
        sink.i32_const(4096);
        sink.local_get(0);
        sink.i64_store(memarg(3, offset));
        sink.i32_const(4096);
        sink.i64_load(memarg(3, offset));
        sink.local_get(0);
        sink.i64_eq();
        sink.br_if(1);
        sink.local_get(1);
        sink.i32_const(1);
        sink.i32_add();
        sink.local_set(1);
        sink.br(0);
        sink.end();
        sink.end();
    }
    sink.end();
    drop(sink);
    function.into_raw_body()
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut value = bytes.len() as u64;
    for index in [0, bytes.len() / 3, bytes.len() / 2, bytes.len().saturating_sub(1)] {
        value = value.rotate_left(13) ^ u64::from(bytes[index]);
    }
    value
}

fn run(groups: u32, iterations: u32, sink: bool) -> u64 {
    let mut result = 0;
    for _ in 0..iterations {
        let body = black_box(if sink { sink_body(groups) } else { enum_body(groups) });
        result ^= black_box(fingerprint(&body));
        black_box(body);
    }
    result
}

#[no_mangle]
pub extern "C" fn encode_enum(groups: u32, iterations: u32) -> u64 {
    run(groups, iterations, false)
}

#[no_mangle]
pub extern "C" fn encode_sink(groups: u32, iterations: u32) -> u64 {
    run(groups, iterations, true)
}

#[no_mangle]
pub extern "C" fn exact_bytes(groups: u32) -> u32 {
    u32::from(enum_body(groups) == sink_body(groups))
}

#[no_mangle]
pub extern "C" fn body_bytes(groups: u32) -> u32 {
    enum_body(groups).len().try_into().unwrap_or(u32::MAX)
}
