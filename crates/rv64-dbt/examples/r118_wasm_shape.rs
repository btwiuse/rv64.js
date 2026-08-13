//! Verify R118's release-Wasm dispatch shape and function-body isolation.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use wasmparser::{KnownCustom, Name, Operator, Parser, Payload, TypeRef, Validator};

#[derive(Default)]
struct OperatorShape {
    operators: u64,
    br_tables: Vec<u32>,
    calls: u64,
    direct_call_targets: BTreeMap<u32, u32>,
    indirect_calls: u64,
}

struct FunctionBody {
    bytes: Vec<u8>,
    locals: Vec<String>,
    canonical_operators: Vec<String>,
    shape: OperatorShape,
}

struct ModuleShape {
    imported_functions: u32,
    names: BTreeMap<u32, String>,
    bodies: BTreeMap<u32, FunctionBody>,
}

fn canonical_operator(operator: &Operator<'_>) -> String {
    match operator {
        Operator::I32Const { .. } => return "I32Const{*}".to_owned(),
        Operator::I64Const { .. } => return "I64Const{*}".to_owned(),
        _ => {}
    }
    let mut rendered = format!("{operator:?}");
    let marker = "offset: ";
    let mut search_from = 0;
    while let Some(relative) = rendered[search_from..].find(marker) {
        let start = search_from + relative + marker.len();
        let end = rendered[start..]
            .find(|character: char| !character.is_ascii_digit())
            .map_or(rendered.len(), |length| start + length);
        rendered.replace_range(start..end, "*");
        search_from = start + 1;
    }
    rendered
}

fn inspect(path: &Path) -> Result<ModuleShape, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Validator::new()
        .validate_all(&bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut imported_functions = 0u32;
    let mut names = BTreeMap::new();
    let mut bodies = BTreeMap::new();
    let mut defined = 0u32;
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.map_err(|error| format!("{}: {error}", path.display()))? {
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|error| format!("{}: {error}", path.display()))?;
                    if matches!(import.ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) {
                        imported_functions += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut shape = OperatorShape::default();
                let locals = body
                    .get_locals_reader()
                    .map_err(|error| format!("{}: {error}", path.display()))?
                    .into_iter()
                    .map(|local| {
                        local
                            .map(|(count, ty)| format!("{count}:{ty:?}"))
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut canonical_operators = Vec::new();
                let mut operators = body
                    .get_operators_reader()
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                while !operators.eof() {
                    let operator = operators
                        .read()
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                    shape.operators += 1;
                    // Rust/LLVM relocates static-data addresses when a changed source
                    // location changes the data section. Ignore only literal values and
                    // memory immediates; retain exact opcode, type, index, branch, and call
                    // structure.
                    canonical_operators.push(canonical_operator(&operator));
                    match operator {
                        Operator::BrTable { targets } => shape.br_tables.push(targets.len() + 1),
                        Operator::Call { function_index }
                        | Operator::ReturnCall { function_index } => {
                            shape.calls += 1;
                            *shape.direct_call_targets.entry(function_index).or_default() += 1;
                        }
                        Operator::CallIndirect { .. }
                        | Operator::ReturnCallIndirect { .. }
                        | Operator::CallRef { .. }
                        | Operator::ReturnCallRef { .. } => shape.indirect_calls += 1,
                        _ => {}
                    }
                }
                bodies.insert(
                    imported_functions + defined,
                    FunctionBody {
                        bytes: body.as_bytes().to_vec(),
                        locals,
                        canonical_operators,
                        shape,
                    },
                );
                defined += 1;
            }
            Payload::CustomSection(section) => {
                if let KnownCustom::Name(reader) = section.as_known() {
                    for subsection in reader {
                        let subsection =
                            subsection.map_err(|error| format!("{}: {error}", path.display()))?;
                        if let Name::Function(map) = subsection {
                            for naming in map {
                                let naming = naming
                                    .map_err(|error| format!("{}: {error}", path.display()))?;
                                names.insert(naming.index, naming.name.to_owned());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ModuleShape {
        imported_functions,
        names,
        bodies,
    })
}

fn step_name(name: &str) -> bool {
    name.contains("rv64_core3cpu") && name.contains("Cpu4step")
}

fn canonical_name(name: &str) -> &str {
    match name.rsplit_once(".llvm.") {
        Some((prefix, suffix)) if suffix.bytes().all(|byte| byte.is_ascii_digit()) => prefix,
        _ => name,
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("r118_wasm_shape: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let control_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: r118_wasm_shape CONTROL CANDIDATE OUTPUT")?;
    let candidate_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: r118_wasm_shape CONTROL CANDIDATE OUTPUT")?;
    let output_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: r118_wasm_shape CONTROL CANDIDATE OUTPUT")?;
    if arguments.next().is_some() {
        return Err("usage: r118_wasm_shape CONTROL CANDIDATE OUTPUT".into());
    }
    let control = inspect(&control_path)?;
    let candidate = inspect(&candidate_path)?;
    if control.imported_functions != candidate.imported_functions {
        return Err("function import count changed".into());
    }
    let changed_names = control
        .names
        .iter()
        .filter_map(|(index, left)| {
            let right = candidate.names.get(index)?;
            (canonical_name(left) != canonical_name(right))
                .then(|| format!("{index}: {left:?} -> {right:?}"))
        })
        .take(20)
        .collect::<Vec<_>>();
    if control.names.len() != candidate.names.len()
        || control.names.keys().ne(candidate.names.keys())
        || !changed_names.is_empty()
    {
        return Err(format!(
            "function names or indices changed (control {}, candidate {}): {}",
            control.names.len(),
            candidate.names.len(),
            changed_names.join("; ")
        ));
    }
    if control.bodies.keys().collect::<Vec<_>>() != candidate.bodies.keys().collect::<Vec<_>>() {
        return Err("defined function population changed".into());
    }

    let step_indices = control
        .names
        .iter()
        .filter_map(|(index, name)| step_name(name).then_some(*index))
        .collect::<Vec<_>>();
    if step_indices.len() != 6 {
        return Err(format!(
            "expected six Cpu::step bodies, found {}",
            step_indices.len()
        ));
    }
    let mut raw_changed_non_step = Vec::new();
    let mut structurally_changed_non_step = Vec::new();
    for (index, control_body) in &control.bodies {
        let candidate_body = candidate
            .bodies
            .get(index)
            .ok_or_else(|| format!("candidate body {index} missing"))?;
        if !step_indices.contains(index) && control_body.bytes != candidate_body.bytes {
            raw_changed_non_step.push(*index);
        }
        if !step_indices.contains(index)
            && (control_body.locals != candidate_body.locals
                || control_body.canonical_operators != candidate_body.canonical_operators)
        {
            structurally_changed_non_step.push(*index);
        }
    }
    if !raw_changed_non_step.is_empty() {
        return Err(format!(
            "non-Cpu::step bodies changed: {raw_changed_non_step:?}"
        ));
    }
    if !structurally_changed_non_step.is_empty() {
        let details = structurally_changed_non_step
            .iter()
            .take(10)
            .map(|index| {
                let left = &control.bodies[index];
                let right = &candidate.bodies[index];
                let mismatch = left
                    .canonical_operators
                    .iter()
                    .zip(&right.canonical_operators)
                    .position(|(left, right)| left != right);
                format!(
                    "{index} {:?}: locals {:?}->{:?}, ops {}->{}, first {:?}: {:?}->{:?}",
                    control.names.get(index),
                    left.locals,
                    right.locals,
                    left.canonical_operators.len(),
                    right.canonical_operators.len(),
                    mismatch,
                    mismatch.and_then(|at| left.canonical_operators.get(at)),
                    mismatch.and_then(|at| right.canonical_operators.get(at)),
                )
            })
            .collect::<Vec<_>>();
        return Err(format!(
            "non-Cpu::step operator structures changed ({}): {}",
            structurally_changed_non_step.len(),
            details.join("; ")
        ));
    }

    let mut report = format!(
        "raw_non_step_body_differences\t{}\n",
        raw_changed_non_step.len()
    );
    report.push_str(
        "index\tname\tcontrol_bytes\tcandidate_bytes\tcontrol_operators\tcandidate_operators\tcontrol_br_tables\tcandidate_br_tables\tcontrol_calls\tcandidate_calls\tcontrol_indirect_calls\tcandidate_indirect_calls\n",
    );
    for index in step_indices {
        let name = control
            .names
            .get(&index)
            .ok_or_else(|| format!("step name {index} missing"))?;
        let control_body = control
            .bodies
            .get(&index)
            .ok_or_else(|| format!("control step body {index} missing"))?;
        let candidate_body = candidate
            .bodies
            .get(&index)
            .ok_or_else(|| format!("candidate step body {index} missing"))?;
        if control_body.shape.br_tables.len() != candidate_body.shape.br_tables.len() + 3 {
            return Err(format!(
                "step {index} did not remove exactly three br_table operators"
            ));
        }
        if control_body.shape.br_tables.get(0..4) != Some(&[4, 9, 9, 9]) {
            return Err(format!(
                "step {index} control compressed prefix is {:?}",
                control_body.shape.br_tables.get(0..4)
            ));
        }
        if candidate_body.shape.br_tables.first() != Some(&25) {
            return Err(format!(
                "step {index} candidate combined table is {:?}",
                candidate_body.shape.br_tables.first()
            ));
        }
        if control_body.shape.calls != candidate_body.shape.calls
            || control_body.shape.indirect_calls != candidate_body.shape.indirect_calls
        {
            let changed_targets = control_body
                .shape
                .direct_call_targets
                .keys()
                .chain(candidate_body.shape.direct_call_targets.keys())
                .filter_map(|target| {
                    let left = control_body
                        .shape
                        .direct_call_targets
                        .get(target)
                        .copied()
                        .unwrap_or(0);
                    let right = candidate_body
                        .shape
                        .direct_call_targets
                        .get(target)
                        .copied()
                        .unwrap_or(0);
                    (left != right).then(|| {
                        format!("{target} {:?} {left}->{right}", control.names.get(target))
                    })
                })
                .collect::<Vec<_>>();
            return Err(format!(
                "step {index} call topology changed: direct {}->{}, indirect {}->{}, changed targets {}",
                control_body.shape.calls,
                candidate_body.shape.calls,
                control_body.shape.indirect_calls,
                candidate_body.shape.indirect_calls,
                changed_targets.join("; "),
            ));
        }
        let list = |values: &[u32]| {
            values
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        writeln!(
            report,
            "{index}\t{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            control_body.bytes.len(),
            candidate_body.bytes.len(),
            control_body.shape.operators,
            candidate_body.shape.operators,
            list(&control_body.shape.br_tables),
            list(&candidate_body.shape.br_tables),
            control_body.shape.calls,
            candidate_body.shape.calls,
            control_body.shape.indirect_calls,
            candidate_body.shape.indirect_calls,
        )
        .expect("write report row");
    }
    fs::write(output_path, report).map_err(|error| error.to_string())?;
    Ok(())
}
