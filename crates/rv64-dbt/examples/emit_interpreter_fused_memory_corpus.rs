//! Emit the frozen R054 interpreter fused-memory opportunity corpus.
//!
//! The control models the current interpreter's successful standard TLB
//! lookup, physical-RAM checks/access, and redundant fused-JIT-TLB publication.
//! The candidate first consumes the already permission/context-checked fused
//! row and directly accesses its proven native/Wasm pointer, falling back to
//! the complete control path on a miss. Both execute the same mixed memory
//! stream and expose memory for independent checksum verification.

use std::env;
use std::fs;
use std::path::PathBuf;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

const TLB_ENTRIES: u32 = 4096;
const ROW_BYTES: u32 = TLB_ENTRIES * 8;
const STD_LOAD_TAG: u32 = 0x1000;
const STD_LOAD_DIFF: u32 = STD_LOAD_TAG + ROW_BYTES;
const STD_STORE_TAG: u32 = STD_LOAD_DIFF + ROW_BYTES;
const STD_STORE_DIFF: u32 = STD_STORE_TAG + ROW_BYTES;
const JIT_LOAD_TAG: u32 = STD_STORE_DIFF + ROW_BYTES;
const JIT_LOAD_OFF: u32 = JIT_LOAD_TAG + ROW_BYTES;
const JIT_STORE_TAG: u32 = JIT_LOAD_OFF + ROW_BYTES;
const JIT_STORE_OFF: u32 = JIT_STORE_TAG + ROW_BYTES;
const CONTEXT: u32 = JIT_STORE_OFF + ROW_BYTES;
const COMPILED_BITMAP: u32 = CONTEXT + 8;
const DIRTY_COUNTER: u32 = COMPILED_BITMAP + 8;
const RAM_POINTER_BASE: u32 = 0x42000;
const DATA_LINEAR: u32 = RAM_POINTER_BASE + 0x2000;
const DATA_BYTES: usize = 512;
const MEMORY_PAGES: u64 = 5;

const VA_BASE: u64 = 0xffff_ffc0_1234_5000;
const PA_BASE: u64 = 0x8000_2000;
const PHYSICAL_RAM_BASE: u64 = 0x8000_0000;
const PHYSICAL_RAM_BYTES: u64 = 0x4000;
const PAGE_MASK: u64 = !0xfff;
const CONTEXT_VALUE: u64 = 0;

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

fn table_address(function: &mut Function, base: u32) {
    function.instruction(&Instruction::I32Const(base as i32));
    function.instruction(&Instruction::LocalGet(7));
    function.instruction(&Instruction::I32Const(3));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Add);
}

fn expected_tag(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(4));
    function.instruction(&Instruction::I64Const(PAGE_MASK as i64));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I32Const(CONTEXT as i32));
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Or);
}

fn compute_index(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(4));
    function.instruction(&Instruction::I64Const(12));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Const((TLB_ENTRIES - 1) as i32));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(7));
}

fn check_not_crossing(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(4));
    function.instruction(&Instruction::I64Const(0xfff));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(8));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(0x1000));
    function.instruction(&Instruction::I64GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
}

fn physical_page_number(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I64Const(12));
    function.instruction(&Instruction::I64ShrU);
}

fn compiled_page_clear(function: &mut Function) {
    function.instruction(&Instruction::I32Const(COMPILED_BITMAP as i32));
    function.instruction(&Instruction::I64Load(I64_MEM));
    physical_page_number(function);
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Eqz);
}

fn physical_range_valid(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I64Const(7));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(
        (PHYSICAL_RAM_BASE + PHYSICAL_RAM_BYTES) as i64,
    ));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::I32And);
}

fn physical_linear_address(function: &mut Function) {
    function.instruction(&Instruction::I32Const(RAM_POINTER_BASE as i32));
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Add);
}

fn direct_linear_address(function: &mut Function, store: bool) {
    function.instruction(&Instruction::LocalGet(4));
    table_address(function, if store { JIT_STORE_OFF } else { JIT_LOAD_OFF });
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I32WrapI64);
}

fn emit_direct_access(function: &mut Function, store: bool) {
    direct_linear_address(function, store);
    if store {
        function.instruction(&Instruction::LocalGet(5));
        function.instruction(&Instruction::I64Store(I64_MEM));
    } else {
        function.instruction(&Instruction::I64Load(I64_MEM));
        function.instruction(&Instruction::LocalSet(5));
    }
}

fn publish_fused_row(function: &mut Function, store: bool) {
    physical_range_valid(function);
    if store {
        compiled_page_clear(function);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::If(BlockType::Empty));
    table_address(function, if store { JIT_STORE_TAG } else { JIT_LOAD_TAG });
    expected_tag(function);
    function.instruction(&Instruction::I64Store(I64_MEM));
    table_address(function, if store { JIT_STORE_OFF } else { JIT_LOAD_OFF });
    function.instruction(&Instruction::I64Const(RAM_POINTER_BASE as i64));
    function.instruction(&Instruction::LocalGet(6));
    function.instruction(&Instruction::I64Const(PHYSICAL_RAM_BASE as i64));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalGet(4));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::I64Store(I64_MEM));
    function.instruction(&Instruction::End);
}

fn emit_control_access(function: &mut Function, store: bool) {
    table_address(function, if store { STD_STORE_TAG } else { STD_LOAD_TAG });
    function.instruction(&Instruction::I64Load(I64_MEM));
    expected_tag(function);
    function.instruction(&Instruction::I64Ne);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(4));
    table_address(function, if store { STD_STORE_DIFF } else { STD_LOAD_DIFF });
    function.instruction(&Instruction::I64Load(I64_MEM));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(6));
    publish_fused_row(function, store);

    if store {
        // Model VirtBus::jit_check_store before its RAM range/access path.
        compiled_page_clear(function);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::I32Const(DIRTY_COUNTER as i32));
        function.instruction(&Instruction::I64Const(1));
        function.instruction(&Instruction::I64Store(I64_MEM));
        function.instruction(&Instruction::End);
    }
    physical_range_valid(function);
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    physical_linear_address(function);
    if store {
        function.instruction(&Instruction::LocalGet(5));
        function.instruction(&Instruction::I64Store(I64_MEM));
    } else {
        function.instruction(&Instruction::I64Load(I64_MEM));
        function.instruction(&Instruction::LocalSet(5));
    }
}

fn emit_access(function: &mut Function, variant: Variant, store: bool, ordinal: u32) {
    function.instruction(&Instruction::I64Const(VA_BASE as i64));
    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I64Const(ordinal as i64));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(63));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(3));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::LocalSet(4));
    if store {
        function.instruction(&Instruction::LocalGet(3));
        function.instruction(&Instruction::I64Const(((ordinal + 1) * 0x9e37) as i64));
        function.instruction(&Instruction::I64Add);
        function.instruction(&Instruction::LocalSet(5));
    }
    check_not_crossing(function);
    compute_index(function);

    match variant {
        Variant::Control => emit_control_access(function, store),
        Variant::Fused => {
            table_address(function, if store { JIT_STORE_TAG } else { JIT_LOAD_TAG });
            function.instruction(&Instruction::I64Load(I64_MEM));
            expected_tag(function);
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            emit_direct_access(function, store);
            function.instruction(&Instruction::Else);
            emit_control_access(function, store);
            function.instruction(&Instruction::End);
        }
    }

    if store {
        function.instruction(&Instruction::LocalGet(3));
        function.instruction(&Instruction::I64Const((ordinal as i64 + 3) * 17));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::LocalSet(3));
    } else {
        function.instruction(&Instruction::LocalGet(3));
        function.instruction(&Instruction::LocalGet(5));
        function.instruction(&Instruction::I64Xor);
        function.instruction(&Instruction::I64Const((ordinal + 1) as i64));
        function.instruction(&Instruction::I64Rotl);
        function.instruction(&Instruction::LocalSet(3));
    }
}

fn driver(variant: Variant) -> Function {
    // Parameter 0 is loop iterations. Locals: remaining i32, cursor/acc/va/
    // value/pa i64, and the direct-map index i32.
    let mut function = Function::new([(1, ValType::I32), (5, ValType::I64), (1, ValType::I32)]);
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalSet(1));
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::LocalSet(2));
    function.instruction(&Instruction::I64Const(0x0123_4567_89ab_cdef));
    function.instruction(&Instruction::LocalSet(3));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::BrIf(1));
    for ordinal in 0..8 {
        emit_access(&mut function, variant, ordinal % 2 == 1, ordinal);
    }
    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::I64Const(8));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(63));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalSet(2));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(1));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(3));
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

fn module(variant: Variant) -> Vec<u8> {
    let index = ((VA_BASE >> 12) as u32) & (TLB_ENTRIES - 1);
    let row_offset = index * 8;
    let tag = (VA_BASE & PAGE_MASK) | CONTEXT_VALUE;
    let physical_diff = PA_BASE.wrapping_sub(VA_BASE);
    let linear_off = u64::from(DATA_LINEAR).wrapping_sub(VA_BASE);

    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], [ValType::I64]);
    module.section(&types);
    let mut functions = FunctionSection::new();
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
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);
    let mut code = CodeSection::new();
    code.function(&driver(variant));
    module.section(&code);

    let mut data = DataSection::new();
    for base in [STD_LOAD_TAG, STD_STORE_TAG, JIT_LOAD_TAG, JIT_STORE_TAG] {
        u64_segment(&mut data, base + row_offset, tag);
    }
    for base in [STD_LOAD_DIFF, STD_STORE_DIFF] {
        u64_segment(&mut data, base + row_offset, physical_diff);
    }
    for base in [JIT_LOAD_OFF, JIT_STORE_OFF] {
        u64_segment(&mut data, base + row_offset, linear_off);
    }
    u64_segment(&mut data, CONTEXT, CONTEXT_VALUE);
    let initial_data: Vec<u8> = (0..DATA_BYTES / 8)
        .flat_map(|index| {
            (0x1020_3040_5060_7080_u64 ^ (index as u64).wrapping_mul(0x9e37_79b9)).to_le_bytes()
        })
        .collect();
    data.active(0, &ConstExpr::i32_const(DATA_LINEAR as i32), initial_data);
    module.section(&data);
    module.finish()
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-interpreter-fused-memory-corpus"));
    fs::create_dir_all(&output).expect("create fused-memory corpus directory");
    for variant in [Variant::Control, Variant::Fused] {
        fs::write(
            output.join(format!("{}.wasm", variant.name())),
            module(variant),
        )
        .expect("write fused-memory corpus module");
    }
}
