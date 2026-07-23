//! Minimal flattened-device-tree (DTB) builder — just enough to describe
//! the TinyEMU-compatible machine to Linux/BBL. FDT v17.

pub struct Fdt {
    struct_: Vec<u8>,
    strings: Vec<u8>,
    /// (offset into strings) cache for property names
    names: Vec<(String, u32)>,
}

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_END: u32 = 9;

impl Fdt {
    pub fn new() -> Fdt {
        Fdt {
            struct_: Vec::new(),
            strings: Vec::new(),
            names: Vec::new(),
        }
    }

    fn u32(&mut self, v: u32) {
        self.struct_.extend_from_slice(&v.to_be_bytes());
    }

    fn name_offset(&mut self, name: &str) -> u32 {
        if let Some((_, off)) = self.names.iter().find(|(n, _)| n == name) {
            return *off;
        }
        let off = self.strings.len() as u32;
        self.strings.extend_from_slice(name.as_bytes());
        self.strings.push(0);
        self.names.push((name.to_string(), off));
        off
    }

    pub fn begin_node(&mut self, name: &str) {
        self.u32(FDT_BEGIN_NODE);
        self.struct_.extend_from_slice(name.as_bytes());
        self.struct_.push(0);
        while self.struct_.len() % 4 != 0 {
            self.struct_.push(0);
        }
    }

    pub fn end_node(&mut self) {
        self.u32(FDT_END_NODE);
    }

    pub fn prop(&mut self, name: &str, data: &[u8]) {
        let off = self.name_offset(name);
        self.u32(FDT_PROP);
        self.u32(data.len() as u32);
        self.u32(off);
        self.struct_.extend_from_slice(data);
        while self.struct_.len() % 4 != 0 {
            self.struct_.push(0);
        }
    }

    pub fn prop_u32(&mut self, name: &str, v: u32) {
        self.prop(name, &v.to_be_bytes());
    }

    pub fn prop_u32s(&mut self, name: &str, vs: &[u32]) {
        let mut d = Vec::new();
        for v in vs {
            d.extend_from_slice(&v.to_be_bytes());
        }
        self.prop(name, &d);
    }

    pub fn prop_u64_pair(&mut self, name: &str, a: u64, b: u64) {
        let mut d = Vec::new();
        d.extend_from_slice(&a.to_be_bytes());
        d.extend_from_slice(&b.to_be_bytes());
        self.prop(name, &d);
    }

    pub fn prop_str(&mut self, name: &str, s: &str) {
        let mut d = s.as_bytes().to_vec();
        d.push(0);
        self.prop(name, &d);
    }

    pub fn prop_strs(&mut self, name: &str, ss: &[&str]) {
        let mut d = Vec::new();
        for s in ss {
            d.extend_from_slice(s.as_bytes());
            d.push(0);
        }
        self.prop(name, &d);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.u32(FDT_END);
        let header_len = 40;
        let rsvmap_len = 16; // one empty entry
        let off_struct = header_len + rsvmap_len;
        let off_strings = off_struct + self.struct_.len();
        let total = off_strings + self.strings.len();

        let mut out = Vec::with_capacity(total);
        for v in [
            0xd00d_feedu32,
            total as u32,
            off_struct as u32,
            off_strings as u32,
            header_len as u32, // off_mem_rsvmap
            17,                // version
            16,                // last_comp_version
            0,                 // boot_cpuid
            self.strings.len() as u32,
            self.struct_.len() as u32,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&[0u8; 16]); // empty reserve map
        out.extend_from_slice(&self.struct_);
        out.extend_from_slice(&self.strings);
        out
    }
}
