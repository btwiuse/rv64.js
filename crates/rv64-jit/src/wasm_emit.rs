//! Minimal WebAssembly binary encoder — just the pieces the JIT needs:
//! one imported memory, one exported function, i32/i64 ops.

pub struct WasmModule {
    code: Vec<u8>,
    n_locals_i64: u32,
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
pub const I64_GE_S: u8 = 0x58;
pub const I64_GE_U: u8 = 0x59;
pub const I64_EXTEND_I32_U: u8 = 0xad;
pub const I32_ADD: u8 = 0x6a;
pub const BLOCK: u8 = 0x02;
pub const LOOP: u8 = 0x03;
pub const IF: u8 = 0x04;
pub const ELSE: u8 = 0x05;
pub const END: u8 = 0x0b;
pub const BR: u8 = 0x0c;
pub const BR_IF: u8 = 0x0d;
pub const RETURN: u8 = 0x0f;
pub const VOID: u8 = 0x40;

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
        }
    }

    // -- instruction stream helpers --

    pub fn op(&mut self, opcode: u8) -> &mut Self {
        self.code.push(opcode);
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

    /// Finish: wrap the instruction stream into a complete wasm module:
    /// - import "env" "memory" (memory 1)
    /// - export "run": [] -> [] with n i64 locals
    pub fn finish(self) -> Vec<u8> {
        let mut m = vec![0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0]; // magic + version

        // type section: one type: [] -> []
        let mut sec = vec![1u8]; // count
        sec.extend_from_slice(&[0x60, 0, 0]);
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

        // code section
        let mut body = Vec::new();
        if self.n_locals_i64 > 0 {
            body.push(1); // one locals-decl group
            uleb(&mut body, self.n_locals_i64 as u64);
            body.push(0x7e); // i64
        } else {
            body.push(0);
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
