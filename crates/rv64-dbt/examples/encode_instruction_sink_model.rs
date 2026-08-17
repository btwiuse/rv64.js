//! Compare wasm-encoder's general `Instruction` enum path with its typed
//! `InstructionSink` path over the same production-shaped instruction stream.
//! This is an opportunity model only; it does not translate or execute a
//! guest and cannot award product performance credit.

use std::hint::black_box;
use std::time::Instant;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

const GROUPS: usize = 4096;
const ITERATIONS: usize = 160;

fn memarg(align: u32, offset: u64) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn enum_body() -> Vec<u8> {
    let mut function = Function::new_with_locals_types([ValType::I64, ValType::I32]);
    for group in 0..GROUPS {
        let offset = ((group & 31) * 8) as u64;
        function.instruction(&Instruction::Block(BlockType::Empty));
        function.instruction(&Instruction::Loop(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Const(group as i64));
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

fn sink_body() -> Vec<u8> {
    let mut function = Function::new_with_locals_types([ValType::I64, ValType::I32]);
    {
        let mut sink = function.instructions();
        for group in 0..GROUPS {
            let offset = ((group & 31) * 8) as u64;
            sink.block(BlockType::Empty);
            sink.loop_(BlockType::Empty);
            sink.local_get(0);
            sink.i64_const(group as i64);
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
    }
    function.into_raw_body()
}

fn time(encode: fn() -> Vec<u8>) -> (f64, usize) {
    let started = Instant::now();
    let mut bytes = 0;
    for _ in 0..ITERATIONS {
        let body = black_box(encode());
        bytes += body.len();
        black_box(body);
    }
    (started.elapsed().as_secs_f64() * 1000.0, bytes)
}

fn main() {
    let enum_once = enum_body();
    let sink_once = sink_body();
    assert_eq!(enum_once, sink_once, "typed sink must preserve exact bytes");

    // Warm both code paths before the order-balanced observations.
    black_box(enum_body());
    black_box(sink_body());
    let mut enum_ms = Vec::new();
    let mut sink_ms = Vec::new();
    for pair in 0..7 {
        let (first, second) = if pair & 1 == 0 {
            (time(enum_body), time(sink_body))
        } else {
            let sink = time(sink_body);
            let enumeration = time(enum_body);
            (enumeration, sink)
        };
        assert_eq!(first.1, second.1);
        enum_ms.push(first.0);
        sink_ms.push(second.0);
    }
    println!(
        "{{\"groups\":{GROUPS},\"instructionsPerGroup\":21,\"iterations\":{ITERATIONS},\"bodyBytes\":{},\"enumMs\":{:?},\"sinkMs\":{:?}}}",
        enum_once.len(), enum_ms, sink_ms,
    );
}
