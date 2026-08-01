//! Minimal WebAssembly binary encoder — just the pieces the JIT needs:
//! one imported memory, one exported function, i32/i64 ops.

pub struct WasmModule {
    code: Vec<u8>,
    n_locals_i64: u32,
    n_locals_i32: u32,
    /// Import `env.tlb_fill: (i64 va, i32 store) -> i64` and call it on a
    /// fused-TLB miss. It resolves to the host module's own exported function,
    /// so the call is wasm->wasm with no JS frame.
    wants_tlb_fill: bool,
    /// Import `env.__indirect_function_table` so the block can tail-call the
    /// next compiled block directly (see emit_chain_exit).
    wants_table: bool,
    /// Import `env.chain_next: (i32 state) -> ()` — the host module's
    /// dispatch-line transfer (see rv64-wasm chain_next). A FUNCTION import
    /// like tlb_fill, deliberately NOT the table: table-importing modules
    /// made every table.set O(importing instances) on V8.
    wants_chain_next: bool,
    /// Optional SECOND function: the shared chain-check helper (() -> i32,
    /// returns the verified table index to tail-call or -1). Emitting the
    /// full chain machinery inline at every trace side exit bloated modules
    /// ~2-3x and kept V8's per-function tiering from ever optimizing large
    /// block populations (tcc ran 2.4x slower with the code EMITTED even
    /// when a kill cell prevented it from ever executing).
    helper: Option<Vec<u8>>,
}

// Opcodes we use.
pub const LOCAL_GET: u8 = 0x20;
pub const LOCAL_SET: u8 = 0x21;
pub const LOCAL_TEE: u8 = 0x22;
pub const I32_CONST: u8 = 0x41;
pub const I64_CONST: u8 = 0x42;
pub const I64_LOAD: u8 = 0x29;
pub const I64_STORE: u8 = 0x37;
pub const I32_WRAP_I64: u8 = 0xa7;
pub const I64_EXTEND_I32_S: u8 = 0xac;
pub const I64_ADD: u8 = 0x7c;
pub const I64_SUB: u8 = 0x7d;
pub const I64_MUL: u8 = 0x7e;
pub const I64_AND: u8 = 0x83;
pub const I64_OR: u8 = 0x84;
pub const I64_XOR: u8 = 0x85;
pub const I64_SHL: u8 = 0x86;
pub const I64_SHR_S: u8 = 0x87;
pub const I64_SHR_U: u8 = 0x88;
pub const I64_EQ: u8 = 0x51;
pub const I64_NE: u8 = 0x52;
pub const I64_LT_S: u8 = 0x53;
pub const I64_LT_U: u8 = 0x54;
pub const I64_GT_U: u8 = 0x56;
pub const I64_GT_S: u8 = 0x55;
pub const I64_GE_S: u8 = 0x59; // (0x58 is le_u — was wrong before)
pub const I64_GE_U: u8 = 0x5a;
// typed i64 memory ops
pub const I64_LOAD8_S: u8 = 0x30;
pub const I64_LOAD8_U: u8 = 0x31;
pub const I64_LOAD16_S: u8 = 0x32;
pub const I64_LOAD16_U: u8 = 0x33;
pub const I64_LOAD32_S: u8 = 0x34;
pub const I64_LOAD32_U: u8 = 0x35;
pub const I64_STORE8: u8 = 0x3c;
pub const I64_STORE16: u8 = 0x3d;
pub const I64_STORE32: u8 = 0x3e;
pub const I64_EQZ: u8 = 0x50;
pub const I32_OR: u8 = 0x72;
pub const I32_XOR: u8 = 0x73;
pub const I32_EQZ: u8 = 0x45;
// f64 arithmetic + reinterpret casts (Phase 2 FP-in-blocks).
pub const F64_ADD: u8 = 0xa0;
pub const F64_SUB: u8 = 0xa1;
pub const F64_MUL: u8 = 0xa2;
pub const F64_DIV: u8 = 0xa3;
pub const F64_EQ: u8 = 0x61;
pub const F64_LT: u8 = 0x63;
pub const F64_LE: u8 = 0x65;
pub const F64_REINTERPRET_I64: u8 = 0xbf;
pub const I64_REINTERPRET_F64: u8 = 0xbd;
pub const UNREACHABLE: u8 = 0x00;
pub const DROP: u8 = 0x1a;
pub const I64_EXTEND_I32_U: u8 = 0xad;
pub const I32_ADD: u8 = 0x6a;
pub const I32_AND: u8 = 0x71;
pub const I32_SHL: u8 = 0x74;
pub const BLOCK: u8 = 0x02;
pub const LOOP: u8 = 0x03;
pub const IF: u8 = 0x04;
pub const ELSE: u8 = 0x05;
pub const END: u8 = 0x0b;
pub const BR: u8 = 0x0c;
pub const BR_IF: u8 = 0x0d;
pub const BR_TABLE: u8 = 0x0e;
pub const RETURN: u8 = 0x0f;
pub const I32_SHR_U: u8 = 0x76;
pub const I32_NE: u8 = 0x47;
pub const I32_LT_S: u8 = 0x48;
pub const I32_GE_U: u8 = 0x4f;
pub const I32_SUB: u8 = 0x6b;
pub const VOID: u8 = 0x40;
// division / remainder (trap-guarded at emission: riscv division never traps)
pub const I64_DIV_S: u8 = 0x7f;
pub const I64_DIV_U: u8 = 0x80;
pub const I64_REM_S: u8 = 0x81;
pub const I64_REM_U: u8 = 0x82;
/// Untyped select: [val1 val2 cond] -> cond != 0 ? val1 : val2.
pub const SELECT: u8 = 0x1b;
// FP conversions / sqrt (FP fast path: FCVT + FSQRT inline)
pub const F64_SQRT: u8 = 0x9f;
pub const F64_GE: u8 = 0x66;
pub const F64_GT: u8 = 0x64;
pub const F64_NE: u8 = 0x62;
pub const I64_TRUNC_F64_S: u8 = 0xb0; // traps out-of-range: range-guarded at emission
pub const F64_CONVERT_I64_S: u8 = 0xb9;
pub const F64_CONVERT_I64_U: u8 = 0xba;
// f32 (the F extension): values live NaN-boxed in the low 32 bits of an f
// register, so every emitter unboxes to i32 and reinterprets.
pub const F32_ADD: u8 = 0x92;
pub const F32_SUB: u8 = 0x93;
pub const F32_MUL: u8 = 0x94;
pub const F32_DIV: u8 = 0x95;
pub const F32_SQRT: u8 = 0x91;
pub const F32_EQ: u8 = 0x5b;
pub const F32_LT: u8 = 0x5d;
pub const F32_LE: u8 = 0x5f;
pub const F32_REINTERPRET_I32: u8 = 0xbe;
pub const I32_REINTERPRET_F32: u8 = 0xbc;
pub const F32_DEMOTE_F64: u8 = 0xb6;
pub const F64_PROMOTE_F32: u8 = 0xbb;
pub const F32_CONVERT_I64_S: u8 = 0xb4;
pub const F32_CONVERT_I64_U: u8 = 0xb5;
pub const I32_TRUNC_F32_S: u8 = 0xa8;
pub const I64_NE_: u8 = 0x52;

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
        if done {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

impl WasmModule {
    pub fn new(n_locals_i64: u32) -> WasmModule {
        WasmModule {
            code: Vec::new(),
            n_locals_i64,
            n_locals_i32: 0,
            wants_tlb_fill: false,
            wants_table: false,
            wants_chain_next: false,
            helper: None,
        }
    }

    pub fn with_locals(n_locals_i64: u32, n_locals_i32: u32) -> WasmModule {
        WasmModule {
            code: Vec::new(),
            n_locals_i64,
            n_locals_i32,
            wants_tlb_fill: false,
            wants_table: false,
            wants_chain_next: false,
            helper: None,
        }
    }

    /// Install the chain-check helper body (locals: 1 i64 then 2 i32; no
    /// params) and return its call index in this module's function index
    /// space. Idempotent.
    pub fn set_helper(&mut self, body: Vec<u8>) -> u32 {
        if self.helper.is_none() {
            self.helper = Some(body);
        }
        // imported funcs (tlb_fill? chain_next?) then the body, then helper.
        u32::from(self.wants_tlb_fill) + u32::from(self.wants_chain_next) + 1
    }

    pub fn has_helper(&self) -> bool {
        self.helper.is_some()
    }

    /// Steal the raw instruction stream (for building helper bodies with the
    /// same emission API).
    pub fn into_code(self) -> Vec<u8> {
        self.code
    }

    /// call a function by index.
    pub fn call(&mut self, idx: u32) -> &mut Self {
        self.code.push(0x10);
        uleb(&mut self.code, idx as u64);
        self
    }

    /// return_call (DIRECT tail call, 0x12): replaces the current frame with
    /// the named function — the intra-batch block-to-block transfer. Direct,
    /// so no table, no import, no signature check at runtime.
    pub fn return_call(&mut self, idx: u32) -> &mut Self {
        self.code.push(0x12);
        uleb(&mut self.code, idx as u64);
        self
    }

    /// Locals counts declared for this module's body (for batch assembly).
    pub fn locals(&self) -> (u32, u32) {
        (self.n_locals_i64, self.n_locals_i32)
    }

    pub fn wants_tlb(&self) -> bool {
        self.wants_tlb_fill
    }

    // -- instruction stream helpers --

    /// Declare the tlb_fill import (function index 0; the block body becomes
    /// function index 1).
    pub fn use_tlb_fill(&mut self) -> &mut Self {
        self.wants_tlb_fill = true;
        self
    }

    pub fn use_table(&mut self) -> &mut Self {
        self.wants_table = true;
        self
    }

    /// Declare the chain_next import. Forces the tlb_fill import too so
    /// call indices stay fixed: tlb_fill = 0, chain_next = 1, body = 2.
    pub fn use_chain_next(&mut self) -> &mut Self {
        self.wants_tlb_fill = true;
        self.wants_chain_next = true;
        self
    }

    /// call $chain_next (function import index 1; see use_chain_next).
    pub fn call_chain_next(&mut self) -> &mut Self {
        self.code.push(0x10);
        uleb(&mut self.code, 1);
        self
    }

    /// return_call_indirect (tail call): pops [args..., i32 func_index] and
    /// replaces the current frame with the callee — the transfer that lets
    /// compiled blocks chain without growing the stack or re-entering the
    /// host dispatch loop.
    pub fn return_call_indirect(&mut self, type_idx: u32) -> &mut Self {
        self.code.push(0x13);
        uleb(&mut self.code, type_idx as u64);
        uleb(&mut self.code, 0); // table 0 (the imported function table)
        self
    }

    /// SIMD (0xfd-prefixed). Emitted ONLY when the host has probed that the
    /// engine validates relaxed SIMD AND that relaxed_madd is FUSED on this
    /// hardware (the spec allows either; only the fused form is bit-exact for
    /// the guest's fmadd).
    pub fn i64x2_splat(&mut self) -> &mut Self {
        self.code.push(0xfd);
        uleb(&mut self.code, 0x12);
        self
    }
    pub fn f64x2_relaxed_madd(&mut self) -> &mut Self {
        self.code.push(0xfd);
        uleb(&mut self.code, 0x107);
        self
    }
    pub fn f64x2_extract_lane0(&mut self) -> &mut Self {
        self.code.push(0xfd);
        uleb(&mut self.code, 0x21);
        self.code.push(0x00);
        self
    }

    /// i32.load (align 2, constant offset).
    pub fn i32_load(&mut self, offset: u64) -> &mut Self {
        self.code.push(0x28);
        uleb(&mut self.code, 2);
        uleb(&mut self.code, offset);
        self
    }

    /// call $tlb_fill — pops (i64 va, i32 store), pushes the i64 offset or -1.
    pub fn call_tlb_fill(&mut self) -> &mut Self {
        self.code.push(0x10);
        uleb(&mut self.code, 0);
        self
    }

    pub fn op(&mut self, opcode: u8) -> &mut Self {
        self.code.push(opcode);
        self
    }

    /// Append a raw ULEB128 immediate (memarg align/offset fields).
    pub fn raw_uleb(&mut self, v: u64) -> &mut Self {
        uleb(&mut self.code, v);
        self
    }

    pub fn i64_const(&mut self, v: i64) -> &mut Self {
        self.code.push(I64_CONST);
        sleb(&mut self.code, v);
        self
    }

    pub fn i32_const(&mut self, v: i32) -> &mut Self {
        self.code.push(I32_CONST);
        sleb(&mut self.code, v as i64);
        self
    }

    pub fn local_get(&mut self, i: u32) -> &mut Self {
        self.code.push(LOCAL_GET);
        uleb(&mut self.code, i as u64);
        self
    }

    pub fn local_set(&mut self, i: u32) -> &mut Self {
        self.code.push(LOCAL_SET);
        uleb(&mut self.code, i as u64);
        self
    }

    // i32-typed local aliases (same opcodes; the type is per the local
    // declaration, this is just intent-documenting sugar).
    pub fn local_get_i32(&mut self, i: u32) -> &mut Self {
        self.local_get(i)
    }
    pub fn local_set_i32(&mut self, i: u32) -> &mut Self {
        self.local_set(i)
    }

    /// i64.load where the address is `<i32 index on stack> + base`, encoded
    /// via the static memarg offset (base is a compile-time constant).
    pub fn i64_load_at(&mut self, base: u64) -> &mut Self {
        self.i64_load(base)
    }

    /// i64.load from linear memory (align 3, given constant offset).
    pub fn i64_load(&mut self, offset: u64) -> &mut Self {
        self.code.push(I64_LOAD);
        uleb(&mut self.code, 3);
        uleb(&mut self.code, offset);
        self
    }

    pub fn i64_store(&mut self, offset: u64) -> &mut Self {
        self.code.push(I64_STORE);
        uleb(&mut self.code, 3);
        uleb(&mut self.code, offset);
        self
    }

    pub fn br(&mut self, depth: u32) -> &mut Self {
        self.code.push(BR);
        uleb(&mut self.code, depth as u64);
        self
    }

    pub fn br_if(&mut self, depth: u32) -> &mut Self {
        self.code.push(BR_IF);
        uleb(&mut self.code, depth as u64);
        self
    }

    /// br_table: pop an i32 index, branch to `targets[index]` (block depths),
    /// or `default` if the index is out of range.
    /// bulk-memory memory.copy: pops [dst_i32, src_i32, len_i32].
    pub fn memory_copy(&mut self) -> &mut Self {
        self.code.push(0xfc);
        uleb(&mut self.code, 10);
        self.code.push(0x00); // dst memidx
        self.code.push(0x00); // src memidx
        self
    }

    pub fn br_table(&mut self, targets: &[u32], default: u32) -> &mut Self {
        self.code.push(BR_TABLE);
        uleb(&mut self.code, targets.len() as u64);
        for &t in targets {
            uleb(&mut self.code, t as u64);
        }
        uleb(&mut self.code, default as u64);
        self
    }

    /// Finish: wrap the instruction stream into a complete wasm module:
    /// - import "env" "memory" (memory 1)
    /// - export "run": [] -> [] with n i64 locals
    pub fn finish(self) -> Vec<u8> {
        let mut m = vec![0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0]; // magic + version

        // type section: one type: (i32) -> [].
        // The parameter is the emulator-state pointer: the host passes it so
        // the pointer visibly escapes into the generated code, which stops
        // LLVM from caching CPU state in registers across block calls.
        let n_types = 1 + u8::from(self.wants_tlb_fill) + u8::from(self.helper.is_some());
        let mut sec = vec![n_types]; // count
        sec.extend_from_slice(&[0x60, 1, 0x7f, 0]);
        if self.wants_tlb_fill {
            // type 1: (i64, i32) -> i64
            sec.extend_from_slice(&[0x60, 2, 0x7e, 0x7f, 1, 0x7e]);
        }
        if self.helper.is_some() {
            // helper type: () -> i32 (last type index)
            sec.extend_from_slice(&[0x60, 0, 1, 0x7f]);
        }
        section(&mut m, 1, &sec);

        // import section: env.memory (+ env.tlb_fill and/or the function
        // table when the body uses them)
        let n_imports = 1
            + u8::from(self.wants_tlb_fill)
            + u8::from(self.wants_chain_next)
            + u8::from(self.wants_table);
        let mut sec = vec![n_imports];
        if self.wants_tlb_fill {
            // imported functions come first in the index space, so declare
            // tlb_fill (func 0) before memory; the body follows the imports.
            sec.push(3);
            sec.extend_from_slice(b"env");
            sec.push(8);
            sec.extend_from_slice(b"tlb_fill");
            sec.extend_from_slice(&[0x00, 0x01]); // func, type 1
        }
        if self.wants_chain_next {
            sec.push(3);
            sec.extend_from_slice(b"env");
            sec.push(10);
            sec.extend_from_slice(b"chain_next");
            sec.extend_from_slice(&[0x00, 0x00]); // func, type 0
        }
        sec.push(3);
        sec.extend_from_slice(b"env");
        sec.push(6);
        sec.extend_from_slice(b"memory");
        sec.extend_from_slice(&[0x02, 0x00, 0x01]); // memory, no-max, min 1
        if self.wants_table {
            sec.push(3);
            sec.extend_from_slice(b"env");
            sec.push(25);
            sec.extend_from_slice(b"__indirect_function_table");
            sec.extend_from_slice(&[0x01, 0x70, 0x00, 0x00]); // table funcref min 0
        }
        section(&mut m, 2, &sec);

        // function section: the body (type 0) + the helper when present.
        if self.helper.is_some() {
            let helper_ty = 1 + u8::from(self.wants_tlb_fill);
            section(&mut m, 3, &[2, 0, helper_ty]);
        } else {
            section(&mut m, 3, &[1, 0]);
        }

        // export section: "run" -> the body function (after func imports)
        let n_func_imports = u8::from(self.wants_tlb_fill) + u8::from(self.wants_chain_next);
        let mut sec = vec![1u8];
        sec.push(3);
        sec.extend_from_slice(b"run");
        sec.extend_from_slice(&[0x00, n_func_imports]);
        section(&mut m, 7, &sec);

        // code section (param is local 0; i64 locals then i32 locals — the
        // declaration order fixes local indices, see lib.rs VA/PAGE/.../IDXB)
        let mut body = Vec::new();
        let mut groups: Vec<(u32, u8)> = Vec::new();
        if self.n_locals_i64 > 0 {
            groups.push((self.n_locals_i64, 0x7e)); // i64
        }
        if self.n_locals_i32 > 0 {
            groups.push((self.n_locals_i32, 0x7f)); // i32
        }
        uleb(&mut body, groups.len() as u64);
        for (count, ty) in groups {
            uleb(&mut body, count as u64);
            body.push(ty);
        }
        body.extend_from_slice(&self.code);
        body.push(END);
        let mut sec = vec![if self.helper.is_some() { 2u8 } else { 1u8 }];
        uleb(&mut sec, body.len() as u64);
        sec.extend_from_slice(&body);
        if let Some(h) = &self.helper {
            // helper body: locals 1 x i64 then 2 x i32.
            let mut hb = Vec::new();
            uleb(&mut hb, 2);
            uleb(&mut hb, 1);
            hb.push(0x7e);
            uleb(&mut hb, 2);
            hb.push(0x7f);
            hb.extend_from_slice(h);
            hb.push(END);
            uleb(&mut sec, hb.len() as u64);
            sec.extend_from_slice(&hb);
        }
        section(&mut m, 10, &sec);

        m
    }
}

fn section(m: &mut Vec<u8>, id: u8, payload: &[u8]) {
    m.push(id);
    uleb(m, payload.len() as u64);
    m.extend_from_slice(payload);
}

/// Assemble a BATCH module: N trace bodies (each with its own locals) in one
/// module, exported "r0".."rN-1". Direct tail calls between bodies transfer
/// in ~2ns with no table import — the design that finally reconciles cheap
/// block chaining with O(1) registration (a shared-table import made every
/// table.set O(importing instances); see the 2026-07-26 chain saga).
/// Function index space: tlb_fill (import 0) then bodies 1..=N — emit links
/// with `return_call(1 + target_member_index)`. The tlb import is always
/// declared so indices are stable whether or not any body uses it.
pub fn finish_batch(bodies: Vec<(Vec<u8>, u32, u32)>) -> Vec<u8> {
    let n = bodies.len();
    let mut m = vec![0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0];

    // types: 0 = (i32) -> (), 1 = tlb (i64, i32) -> i64
    let sec_types: Vec<u8> = vec![2, 0x60, 1, 0x7f, 0, 0x60, 2, 0x7e, 0x7f, 1, 0x7e];
    section(&mut m, 1, &sec_types);

    // imports: tlb_fill (func 0), memory
    let mut sec = vec![2u8];
    sec.push(3);
    sec.extend_from_slice(b"env");
    sec.push(8);
    sec.extend_from_slice(b"tlb_fill");
    sec.extend_from_slice(&[0x00, 0x01]);
    sec.push(3);
    sec.extend_from_slice(b"env");
    sec.push(6);
    sec.extend_from_slice(b"memory");
    sec.extend_from_slice(&[0x02, 0x00, 0x01]);
    section(&mut m, 2, &sec);

    // functions: n bodies of type 0
    let mut sec = Vec::new();
    uleb(&mut sec, n as u64);
    for _ in 0..n {
        sec.push(0);
    }
    section(&mut m, 3, &sec);

    // exports: "r<i>" -> func 1 + i
    let mut sec = Vec::new();
    uleb(&mut sec, n as u64);
    for i in 0..n {
        let name = format!("r{i}");
        uleb(&mut sec, name.len() as u64);
        sec.extend_from_slice(name.as_bytes());
        sec.push(0x00);
        uleb(&mut sec, (1 + i) as u64);
    }
    section(&mut m, 7, &sec);

    // code: each body with its own locals (i64 group then i32 group)
    let mut sec = Vec::new();
    uleb(&mut sec, n as u64);
    for (code, n64, n32) in &bodies {
        let mut body = Vec::new();
        let mut groups: Vec<(u32, u8)> = Vec::new();
        if *n64 > 0 {
            groups.push((*n64, 0x7e));
        }
        if *n32 > 0 {
            groups.push((*n32, 0x7f));
        }
        uleb(&mut body, groups.len() as u64);
        for (count, ty) in groups {
            uleb(&mut body, count as u64);
            body.push(ty);
        }
        body.extend_from_slice(code);
        body.push(END);
        uleb(&mut sec, body.len() as u64);
        sec.extend_from_slice(&body);
    }
    section(&mut m, 10, &sec);

    m
}
