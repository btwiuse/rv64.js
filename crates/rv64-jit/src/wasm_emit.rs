//! Minimal WebAssembly binary encoder — just the pieces the JIT needs:
//! one imported memory, one exported function, i32/i64 ops.

pub struct WasmModule {
    code: Vec<u8>,
    n_locals_i64: u32,
    n_locals_i32: u32,
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
        }
    }

    pub fn with_locals(n_locals_i64: u32, n_locals_i32: u32) -> WasmModule {
        WasmModule {
            code: Vec::new(),
            n_locals_i64,
            n_locals_i32,
        }
    }

    // -- instruction stream helpers --

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
        let mut sec = vec![1u8]; // count
        sec.extend_from_slice(&[0x60, 1, 0x7f, 0]);
        section(&mut m, 1, &sec);

        // import section: env.memory, min 1 page
        let mut sec = vec![1u8];
        sec.push(3);
        sec.extend_from_slice(b"env");
        sec.push(6);
        sec.extend_from_slice(b"memory");
        sec.extend_from_slice(&[0x02, 0x00, 0x01]); // memory, no-max, min 1
        section(&mut m, 2, &sec);

        // function section: 1 function of type 0
        section(&mut m, 3, &[1, 0]);

        // export section: "run" -> func 0
        let mut sec = vec![1u8];
        sec.push(3);
        sec.extend_from_slice(b"run");
        sec.extend_from_slice(&[0x00, 0x00]);
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
        let mut sec = vec![1u8];
        uleb(&mut sec, body.len() as u64);
        sec.extend_from_slice(&body);
        section(&mut m, 10, &sec);

        m
    }
}

fn section(m: &mut Vec<u8>, id: u8, payload: &[u8]) {
    m.push(id);
    uleb(m, payload.len() as u64);
    m.extend_from_slice(payload);
}
