//! Emit the frozen R055 interpreter fused-fetch opportunity corpus.
//!
//! The control models a successful standard execute-TLB lookup followed by
//! physical-RAM classification and a halfword load. The candidate consumes a
//! one-page execute-context-tagged pointer capability and falls back to the
//! complete control path on a miss. Both execute the same compressed/32-bit
//! stream; `edge` additionally crosses a page at a 32-bit instruction's high
//! halfword and switches back, proving refill behavior independently of the
//! same-page throughput loop.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

const TLB_ENTRIES: u32 = 4096;
const ROW_BYTES: u32 = TLB_ENTRIES * 8;
const STD_FETCH_TAG: u32 = 0x1000;
const STD_FETCH_DIFF: u32 = STD_FETCH_TAG + ROW_BYTES;
const FUSED_FETCH_TAG: u32 = STD_FETCH_DIFF + ROW_BYTES;
const FUSED_FETCH_OFF: u32 = FUSED_FETCH_TAG + 8;
const FETCH_CONTEXT: u32 = FUSED_FETCH_OFF + 8;
const RAM_POINTER_BASE: u32 = 0x20000;
const DATA_LINEAR: u32 = RAM_POINTER_BASE + 0x2000;
const DATA_BYTES: usize = 0x2000;
const MEMORY_PAGES: u64 = 3;

const VA_BASE: u64 = 0xffff_ffc0_2345_6000;
const PA_BASE: u64 = 0x8000_2000;
const PHYSICAL_RAM_BASE: u64 = 0x8000_0000;
const PHYSICAL_RAM_BYTES: u64 = 0x6000;
const PAGE_MASK: u64 = !0xfff;
const CONTEXT_VALUE: u64 = 1; // Supervisor execute context in the tag's low bits.

const HOT_INSTRUCTIONS: &[(u64, u16, Option<u16>)] = &[
    (0x000, 0x0001, None),
    (0x002, 0x0013, Some(0x0101)),
    (0x006, 0x0005, None),
    (0x008, 0x0073, Some(0x0202)),
    (0x00c, 0x0009, None),
    (0x00e, 0x0013, Some(0x0303)),
    (0x012, 0x000d, None),
    (0x014, 0x006f, Some(0x0404)),
];

#[derive(Clone, Copy)]
enum Variant {
    Control,
    Fused,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Fused => "fused",
        }
    }
}

const I64_MEM: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: 0,
};
const I16_MEM: MemArg = MemArg {
    offset: 0,
    align: 1,
    memory_index: 0,
};

// Function layout: parameter 0, remaining i32, accumulator/VA/value/PA i64,
// and direct-map index i32.
const REMAINING: u32 = 1;
const ACCUMULATOR: u32 = 2;
const VA: u32 = 3;
const VALUE: u32 = 4;
const PA: u32 = 5;
const INDEX: u32 = 6;

fn table_address(function: &mut Function, base: u32) {
    function.instruction(&Instruction::I32Const(base as i32));
    function.instruction(&Instruction::LocalGet(INDEX));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
}

fn expected_tag(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(VA));
    function.instruction(&Instruction::I64Const(PAGE_MASK as i64));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I32Const(FETCH_CONTEXT as i32));
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Or);
}

fn compute_index(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(VA));
    function.instruction(&Instruction::I64Const(12));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const((TLB_ENTRIES - 1) as i32));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(INDEX));
}

fn fill_standard_row(function: &mut Function) {
    table_address(function, STD_FETCH_TAG);
    expected_tag(function);
    function.instruction(&Instruction::I64Store(I64_MEM));
    table_address(function, STD_FETCH_DIFF);
    function.instruction(&Instruction::I64Const(PA_BASE.wrapping_sub(VA_BASE) as i64));
    function.instruction(&Instruction::I64Store(I64_MEM));
}

fn translate_standard(function: &mut Function) {
    compute_index(function);
    table_address(function, STD_FETCH_TAG);
    function.instruction(&Instruction::I64Load(I64_MEM));
    expected_tag(function);
    function.instruction(&Instruction::I64Ne);
    function.instruction(&Instruction::If(BlockType::Empty));
    fill_standard_row(function);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(VA));
    table_address(function, STD_FETCH_DIFF);
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(PA));
}

fn physical_range_check(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(PA));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::LocalGet(PA));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(
        (PHYSICAL_RAM_BASE + PHYSICAL_RAM_BYTES) as i64,
    ));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
}

fn physical_linear_address(function: &mut Function) {
    function.instruction(&Instruction::I32Const(RAM_POINTER_BASE as i32));
    function.instruction(&Instruction::LocalGet(PA));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Add);
}

fn publish_fused(function: &mut Function) {
    function.instruction(&Instruction::I32Const(FUSED_FETCH_TAG as i32));
    expected_tag(function);
    function.instruction(&Instruction::I64Store(I64_MEM));
    function.instruction(&Instruction::I32Const(FUSED_FETCH_OFF as i32));
    function.instruction(&Instruction::I64Const(RAM_POINTER_BASE as i64));
    function.instruction(&Instruction::LocalGet(PA));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalGet(VA));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I64Store(I64_MEM));
}

fn standard_fetch(function: &mut Function, publish: bool) {
    translate_standard(function);
    physical_range_check(function);
    if publish {
        publish_fused(function);
    }
    physical_linear_address(function);
    function.instruction(&Instruction::I64Load16U(I16_MEM));
    function.instruction(&Instruction::LocalSet(VALUE));
}

fn direct_fetch(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(VA));
    function.instruction(&Instruction::I32Const(FUSED_FETCH_OFF as i32));
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I64Load16U(I16_MEM));
    function.instruction(&Instruction::LocalSet(VALUE));
}

fn emit_fetch(function: &mut Function, variant: Variant) {
    match variant {
        Variant::Control => standard_fetch(function, false),
        Variant::Fused => {
            function.instruction(&Instruction::I32Const(FUSED_FETCH_TAG as i32));
            function.instruction(&Instruction::I64Load(I64_MEM));
            expected_tag(function);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            direct_fetch(function);
            function.instruction(&Instruction::Else);
            standard_fetch(function, true);
            function.instruction(&Instruction::End);
        }
    }
}

fn mix_value(function: &mut Function, rotation: i64) {
    function.instruction(&Instruction::LocalGet(ACCUMULATOR));
    function.instruction(&Instruction::LocalGet(VALUE));
    function.instruction(&Instruction::I64Xor);
    function.instruction(&Instruction::I64Const(rotation));
    function.instruction(&Instruction::I64Rotl);
    function.instruction(&Instruction::LocalSet(ACCUMULATOR));
}

fn emit_instruction(function: &mut Function, variant: Variant, offset: u64, ordinal: u32) {
    function.instruction(&Instruction::I64Const(VA_BASE.wrapping_add(offset) as i64));
    function.instruction(&Instruction::LocalSet(VA));
    emit_fetch(function, variant);
    mix_value(function, i64::from(ordinal + 1));
    function.instruction(&Instruction::LocalGet(VALUE));
    function.instruction(&Instruction::I64Const(3));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(3));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(VA));
    function.instruction(&Instruction::I64Const(2));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(VA));
    emit_fetch(function, variant);
    mix_value(function, i64::from(ordinal + 9));
    function.instruction(&Instruction::End);
}

fn locals() -> Function {
    Function::new([(1, ValType::I32), (4, ValType::I64), (1, ValType::I32)])
}

fn hot_driver(variant: Variant) -> Function {
    let mut function = locals();
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::I64Const(0x0123_4567_89ab_cdef));
    function.instruction(&Instruction::LocalSet(ACCUMULATOR));
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));
    for (ordinal, (offset, _, _)) in HOT_INSTRUCTIONS.iter().enumerate() {
        emit_instruction(&mut function, variant, *offset, ordinal as u32);
    }
    function.instruction(&Instruction::LocalGet(REMAINING));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(REMAINING));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(ACCUMULATOR));
    function.instruction(&Instruction::End);
    function
}

fn edge_driver(variant: Variant) -> Function {
    let mut function = locals();
    function.instruction(&Instruction::I64Const(0xfedc_ba98_7654_3210_u64 as i64));
    function.instruction(&Instruction::LocalSet(ACCUMULATOR));
    emit_instruction(&mut function, variant, 0xffe, 0);
    emit_instruction(&mut function, variant, 0x1020, 1);
    emit_instruction(&mut function, variant, 0x0040, 2);
    function.instruction(&Instruction::LocalGet(ACCUMULATOR));
    function.instruction(&Instruction::End);
    function
}

fn u64_segment(data: &mut DataSection, address: u32, value: u64) {
    data.active(
        0,
        &ConstExpr::i32_const(address as i32),
        value.to_le_bytes(),
    );
}

fn put_halfword(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn module(variant: Variant) -> Vec<u8> {
    let physical_diff = PA_BASE.wrapping_sub(VA_BASE);
    let linear_off = u64::from(DATA_LINEAR).wrapping_sub(VA_BASE);
    let tag = (VA_BASE & PAGE_MASK) | CONTEXT_VALUE;

    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], [ValType::I64]);
    module.section(&types);
    let mut functions = FunctionSection::new();
    functions.function(0);
    functions.function(0);
    module.section(&functions);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: MEMORY_PAGES,
        maximum: Some(MEMORY_PAGES),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 0);
    exports.export("edge", ExportKind::Func, 1);
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);
    let mut code = CodeSection::new();
    code.function(&hot_driver(variant));
    code.function(&edge_driver(variant));
    module.section(&code);

    let mut data = DataSection::new();
    for page in 0..2_u64 {
        let va = VA_BASE + page * 0x1000;
        let index = ((va >> 12) as u32) & (TLB_ENTRIES - 1);
        u64_segment(
            &mut data,
            STD_FETCH_TAG + index * 8,
            (va & PAGE_MASK) | CONTEXT_VALUE,
        );
        u64_segment(&mut data, STD_FETCH_DIFF + index * 8, physical_diff);
    }
    u64_segment(&mut data, FUSED_FETCH_TAG, tag);
    u64_segment(&mut data, FUSED_FETCH_OFF, linear_off);
    u64_segment(&mut data, FETCH_CONTEXT, CONTEXT_VALUE);

    let mut instruction_bytes: Vec<u8> = (0..DATA_BYTES)
        .map(|index| (index as u8).wrapping_mul(37).wrapping_add(11))
        .collect();
    for &(offset, low, high) in HOT_INSTRUCTIONS {
        put_halfword(&mut instruction_bytes, offset as usize, low);
        if let Some(high) = high {
            put_halfword(&mut instruction_bytes, offset as usize + 2, high);
        }
    }
    put_halfword(&mut instruction_bytes, 0x040, 0x0011);
    put_halfword(&mut instruction_bytes, 0xffe, 0x0013);
    put_halfword(&mut instruction_bytes, 0x1000, 0x5a5a);
    put_halfword(&mut instruction_bytes, 0x1020, 0x0021);
    data.active(
        0,
        &ConstExpr::i32_const(DATA_LINEAR as i32),
        instruction_bytes,
    );
    module.section(&data);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-interpreter-fused-fetch-corpus"));
    fs::create_dir_all(&output).expect("create fused-fetch corpus directory");
    for variant in [Variant::Control, Variant::Fused] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant),
        )
        .expect("write fused-fetch corpus module");
    }
}
