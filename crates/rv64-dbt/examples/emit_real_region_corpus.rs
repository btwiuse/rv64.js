//! Freeze multi-entry modules from real compiler-produced RV64 ELF text.
//!
//! This is deliberately a small, dependency-free ELF reader: the benchmark
//! must preserve the exact input bytes, entry addresses, region geometry, and
//! generated modules without involving a target linker or disassembler.  It
//! accepts `label=path` inputs after the output directory.  With no explicit
//! inputs it uses the locally provisioned rvbench and Alpine musl artifacts.

use rv64_dbt::{
    discover_page_leaders_ext, emittable_at, is_loop_at, scan_basic_block_shapes_pub,
    scan_regs_super_pub, translate_superblock_sparse_state, JitLayout, MultiEntryState,
    ReservationCapability, SystemMemory,
};
use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const PAGE_SIZE: usize = 4096;

#[derive(Clone, Debug)]
struct Section {
    kind: u32,
    flags: u64,
    address: u64,
    offset: u64,
    size: u64,
    link: u32,
    entry_size: u64,
}

#[derive(Clone, Debug)]
struct Function {
    name: String,
    address: u64,
    size: u64,
}

#[derive(Debug)]
struct Elf<'a> {
    bytes: &'a [u8],
    entry: u64,
    sections: Vec<Section>,
    functions: Vec<Function>,
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("ELF u16 outside file at {offset}"))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("ELF u32 outside file at {offset}"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("ELF u64 outside file at {offset}"))?;
    Ok(u64::from_le_bytes(value.try_into().unwrap()))
}

fn range(bytes: &[u8], offset: u64, size: u64) -> Result<&[u8], String> {
    let start = usize::try_from(offset).map_err(|_| "ELF offset is too large")?;
    let len = usize::try_from(size).map_err(|_| "ELF range is too large")?;
    bytes
        .get(start..start.checked_add(len).ok_or("ELF range overflow")?)
        .ok_or_else(|| format!("ELF range {offset:#x}+{size:#x} is outside file"))
}

fn string_at(table: &[u8], offset: u32) -> String {
    let Some(bytes) = table.get(offset as usize..) else {
        return String::new();
    };
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl<'a> Elf<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.get(..16) != Some(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]) {
            return Err("input is not a little-endian ELF64 file".into());
        }
        if u16_at(bytes, 18)? != 243 {
            return Err("ELF machine is not RISC-V".into());
        }
        let entry = u64_at(bytes, 24)?;
        let section_offset =
            usize::try_from(u64_at(bytes, 40)?).map_err(|_| "section table offset is too large")?;
        let section_entry_size = usize::from(u16_at(bytes, 58)?);
        let section_count = usize::from(u16_at(bytes, 60)?);
        if section_entry_size < 64 || section_count == 0 {
            return Err("ELF has no usable section table".into());
        }

        let mut sections = Vec::with_capacity(section_count);
        for index in 0..section_count {
            let offset = section_offset
                .checked_add(
                    index
                        .checked_mul(section_entry_size)
                        .ok_or("section overflow")?,
                )
                .ok_or("section overflow")?;
            sections.push(Section {
                kind: u32_at(bytes, offset + 4)?,
                flags: u64_at(bytes, offset + 8)?,
                address: u64_at(bytes, offset + 16)?,
                offset: u64_at(bytes, offset + 24)?,
                size: u64_at(bytes, offset + 32)?,
                link: u32_at(bytes, offset + 40)?,
                entry_size: u64_at(bytes, offset + 56)?,
            });
        }

        let mut functions = Vec::new();
        for section in &sections {
            // SHT_SYMTAB or SHT_DYNSYM.
            if !matches!(section.kind, 2 | 11) || section.entry_size < 24 {
                continue;
            }
            let Some(strings) = sections.get(section.link as usize) else {
                continue;
            };
            let symbols = range(bytes, section.offset, section.size)?;
            let string_table = range(bytes, strings.offset, strings.size)?;
            for symbol in symbols.chunks_exact(section.entry_size as usize) {
                let info = symbol[4];
                let section_index = u16::from_le_bytes([symbol[6], symbol[7]]) as usize;
                if info & 0x0f != 2 || section_index == 0 || section_index >= sections.len() {
                    continue;
                }
                let address = u64::from_le_bytes(symbol[8..16].try_into().unwrap());
                let size = u64::from_le_bytes(symbol[16..24].try_into().unwrap());
                let owner = &sections[section_index];
                if owner.flags & 0x4 == 0
                    || address < owner.address
                    || address >= owner.address.saturating_add(owner.size)
                {
                    continue;
                }
                let name_offset = u32::from_le_bytes(symbol[..4].try_into().unwrap());
                let name = string_at(string_table, name_offset);
                if !name.is_empty() {
                    functions.push(Function {
                        name,
                        address,
                        size,
                    });
                }
            }
        }
        functions.sort_by_key(|function| (function.address, function.name.clone()));
        functions.dedup_by_key(|function| function.address);
        Ok(Self {
            bytes,
            entry,
            sections,
            functions,
        })
    }

    fn executable_at(&self, address: u64) -> bool {
        self.sections.iter().any(|section| {
            section.kind == 1
                && section.flags & 0x4 != 0
                && address >= section.address
                && address < section.address.saturating_add(section.size)
        })
    }

    fn page_window(&self, first_page: u64, pages: usize) -> Result<Vec<u8>, String> {
        let mut output = vec![0; pages * PAGE_SIZE];
        let end = first_page.saturating_add(output.len() as u64);
        for section in &self.sections {
            if section.kind != 1 || section.flags & 0x4 == 0 {
                continue;
            }
            let section_end = section.address.saturating_add(section.size);
            let lo = first_page.max(section.address);
            let hi = end.min(section_end);
            if lo >= hi {
                continue;
            }
            let source_offset = section.offset.saturating_add(lo - section.address);
            let source = range(self.bytes, source_offset, hi - lo)?;
            let destination = usize::try_from(lo - first_page).map_err(|_| "page offset")?;
            output[destination..destination + source.len()].copy_from_slice(source);
        }
        Ok(output)
    }

    fn representative_functions(&self, capacity: usize) -> Vec<Function> {
        let mut selected = Vec::new();
        if self.executable_at(self.entry) {
            selected.push(Function {
                name: "_elf_entry".into(),
                address: self.entry,
                size: 0,
            });
        }
        let mut by_size = self.functions.clone();
        by_size.sort_by_key(|function| (std::cmp::Reverse(function.size), function.address));
        if let Some(main) = by_size.iter().find(|function| function.name == "main") {
            if selected.iter().all(|item| item.address != main.address) {
                selected.push(main.clone());
            }
        }
        for function in by_size {
            if selected.len() >= capacity {
                break;
            }
            if function.size < 64
                || selected
                    .iter()
                    .any(|item| item.address & !0xfff == function.address & !0xfff)
            {
                continue;
            }
            selected.push(function);
        }
        selected.truncate(capacity);
        selected
    }
}

fn system_layout() -> JitLayout {
    JitLayout {
        x_base: 0,
        pc_addr: 256,
        mem: None,
        sys: Some(SystemMemory::fused_4k(
            4096, 6144, 8192, 10_240, 12_288, 255, false,
        )),
        mem_profile: None,
        reg_stress: false,
        reg_profile_base: 0,
        structured_profile: None,
        multi_latch: false,
        retired_addr: 264,
        f_base: 512,
        fcsr_addr: 768,
        reservation: Some(ReservationCapability::System),
        fuel_addr: 776,
        mstatus_addr: 784,
        copystat_addr: 792,
        chain_off_addr: 800,
        batch_base_addr: 808,
        dispatch_base: 12_288,
        dispatch_mask: 4095,
        map_gen_addr: 824,
        chain_hops_addr: 848,
        ic_miss_owner_addr: 832,
        ic_miss_target_addr: 840,
        pic_code_base: None,
    }
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("emit_real_region_corpus: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let output = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/jit-real-region-corpus"));
    let explicit: Vec<_> = arguments.collect();
    let inputs: Vec<(String, PathBuf)> = if explicit.is_empty() {
        [
            ("rvbench", "target/bench/xbench/rvbench.rv64"),
            (
                "alpine-musl",
                "target/bench/alpine-riscv64/lib/ld-musl-riscv64.so.1",
            ),
        ]
        .into_iter()
        .filter(|&(_, path)| Path::new(path).is_file())
        .map(|(label, path)| (label.to_string(), PathBuf::from(path)))
        .collect()
    } else {
        explicit
            .into_iter()
            .map(|value| {
                let value = value.to_string_lossy();
                let (label, path) = value
                    .split_once('=')
                    .ok_or_else(|| format!("input must be label=path: {value}"))?;
                Ok((safe_name(label), PathBuf::from(path)))
            })
            .collect::<Result<_, String>>()?
    };
    if inputs.is_empty() {
        return Err("no RV64 ELF inputs were found; pass label=path arguments".into());
    }
    fs::create_dir_all(&output).map_err(|error| error.to_string())?;

    let layout = system_layout();
    // Isolate page count at the selected small leader policy, then sweep the
    // leader cap at the production three-page envelope.
    let geometries = [
        (1usize, 32usize),
        (2, 32),
        (3, 32),
        (3, 64),
        (3, 128),
        (3, 256),
        (3, 512),
    ];
    let modes = [
        ("eager", MultiEntryState::RegisterEager),
        ("lazy", MultiEntryState::RegisterLazy),
        ("direct", MultiEntryState::RegisterDirect),
        ("memory", MultiEntryState::Memory),
        ("tailcall", MultiEntryState::MemoryTailCall),
        ("structured", MultiEntryState::RegisterStructured),
    ];
    let mut manifest = String::from(
        "id\tworkload\tsource\tfunction\tseed\tpages\tleader_cap\tentries\tread_x\twrite_x\tread_f\twrite_f\tmode\twasm\tbytes\tspan_lo\tspan_hi\tuses_fp\n",
    );
    let mut shape_manifest = String::from(
        "id\tmember\tpc\tread_x\twrite_x\tread_f\twrite_f\tread_fcsr\twrite_fcsr\ti32_values\ti64_values\tretired\tsuccessors\n",
    );
    let mut region_id = 0usize;
    let mut module_count = 0usize;

    for (workload, path) in inputs {
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let elf = Elf::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        let representatives = elf.representative_functions(4);
        if representatives.is_empty() {
            return Err(format!(
                "{} has no representative executable entries",
                path.display()
            ));
        }

        for function in representatives {
            for &(pages, leader_cap) in &geometries {
                let first_page = function.address & !0xfff;
                let code = elf.page_window(first_page, pages)?;
                let page_vas: Vec<u64> = (0..pages)
                    .map(|index| first_page + (index * PAGE_SIZE) as u64)
                    .collect();
                let (leaders, backedges) = discover_page_leaders_ext(
                    &code,
                    first_page,
                    first_page,
                    (pages * PAGE_SIZE) as u64,
                    &[function.address],
                    leader_cap,
                );
                let entries: Vec<u64> = leaders
                    .into_iter()
                    .filter(|&entry| {
                        emittable_at(&code, first_page, entry, layout)
                            && (!backedges.contains(&entry)
                                || !is_loop_at(&code, first_page, entry, layout))
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if entries.len() < 2 {
                    continue;
                }
                let (read_x, write_x, read_f, write_f) = scan_regs_super_pub(
                    &code,
                    first_page,
                    first_page + (pages * PAGE_SIZE) as u64,
                    &entries,
                    &layout,
                );
                let capture_id = format!("r{region_id:03}");
                region_id += 1;
                let shapes = scan_basic_block_shapes_pub(
                    &code,
                    first_page,
                    first_page + (pages * PAGE_SIZE) as u64,
                    &entries,
                    &layout,
                );
                if shapes.len() != entries.len() {
                    return Err(format!(
                        "{workload}/{} pages={pages} cap={leader_cap}: {} entries but {} shapes",
                        function.name,
                        entries.len(),
                        shapes.len(),
                    ));
                }
                for (member, shape) in shapes.iter().enumerate() {
                    let successors = shape
                        .successors
                        .iter()
                        .map(|pc| format!("{pc:x}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    writeln!(
                        shape_manifest,
                        "{capture_id}\t{member}\t{:x}\t{:08x}\t{:08x}\t{:08x}\t{:08x}\t{}\t{}\t{}\t{}\t{}\t{successors}",
                        shape.entry_pc,
                        shape.read_x,
                        shape.write_x,
                        shape.read_f,
                        shape.write_f,
                        u8::from(shape.read_fcsr),
                        u8::from(shape.write_fcsr),
                        shape.i32_values,
                        shape.i64_values,
                        shape.retired,
                    )
                    .unwrap();
                }
                for &(mode_name, mode) in &modes {
                    let region = translate_superblock_sparse_state(
                        &code,
                        &page_vas,
                        &entries,
                        layout,
                        mode,
                    )
                    .ok_or_else(|| {
                        format!(
                            "translation failed for {workload}/{} pages={pages} cap={leader_cap} mode={mode_name}",
                            function.name
                        )
                    })?;
                    let file_name = format!("{capture_id}-{mode_name}.wasm");
                    fs::write(output.join(&file_name), &region.wasm)
                        .map_err(|error| error.to_string())?;
                    writeln!(
                        manifest,
                        "{capture_id}\t{}\t{}\t{}\t0x{:x}\t{pages}\t{leader_cap}\t{}\t{}\t{}\t{}\t{}\t{mode_name}\t{file_name}\t{}\t0x{:x}\t0x{:x}\t{}",
                        safe_name(&workload),
                        path.display(),
                        safe_name(&function.name),
                        function.address,
                        entries.len(),
                        read_x.count_ones(),
                        write_x.count_ones(),
                        read_f.count_ones(),
                        write_f.count_ones(),
                        region.wasm.len(),
                        region.span.0,
                        region.span.1,
                        u8::from(region.uses_fp),
                    )
                    .unwrap();
                    module_count += 1;
                }
            }
        }
    }

    if module_count == 0 {
        return Err("no input produced a multi-entry region".into());
    }
    fs::write(output.join("manifest.tsv"), manifest).map_err(|error| error.to_string())?;
    fs::write(output.join("member-shapes.tsv"), shape_manifest)
        .map_err(|error| error.to_string())?;
    eprintln!(
        "froze {module_count} modules from {region_id} matched real-code regions in {}",
        output.display()
    );
    Ok(())
}
