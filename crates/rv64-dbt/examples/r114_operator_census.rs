//! Decode paired real-region structured modules for the R114 shape proof.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use wasmparser::{Operator, Parser, Payload, Validator};

#[derive(Clone, Copy, Default)]
struct Counts {
    bytes: u64,
    functions: u64,
    operators: u64,
    local_get: u64,
    local_set: u64,
    local_tee: u64,
    if_: u64,
    br: u64,
    br_if: u64,
    end: u64,
    call: u64,
    return_: u64,
}

impl Counts {
    fn add(&mut self, other: Self) {
        self.bytes += other.bytes;
        self.functions += other.functions;
        self.operators += other.operators;
        self.local_get += other.local_get;
        self.local_set += other.local_set;
        self.local_tee += other.local_tee;
        self.if_ += other.if_;
        self.br += other.br;
        self.br_if += other.br_if;
        self.end += other.end;
        self.call += other.call;
        self.return_ += other.return_;
    }

    fn fields(self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.bytes,
            self.functions,
            self.operators,
            self.local_get,
            self.local_set,
            self.local_tee,
            self.if_,
            self.br,
            self.br_if,
            self.end,
            self.call,
            self.return_,
        )
    }
}

fn census(path: &Path) -> Result<Counts, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Validator::new()
        .validate_all(&bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut counts = Counts {
        bytes: bytes.len() as u64,
        ..Counts::default()
    };
    for payload in Parser::new(0).parse_all(&bytes) {
        let payload = payload.map_err(|error| format!("{}: {error}", path.display()))?;
        let Payload::CodeSectionEntry(body) = payload else {
            continue;
        };
        counts.functions += 1;
        let mut operators = body
            .get_operators_reader()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        while !operators.eof() {
            let operator = operators
                .read()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            counts.operators += 1;
            match operator {
                Operator::LocalGet { .. } => counts.local_get += 1,
                Operator::LocalSet { .. } => counts.local_set += 1,
                Operator::LocalTee { .. } => counts.local_tee += 1,
                Operator::If { .. } => counts.if_ += 1,
                Operator::Br { .. } => counts.br += 1,
                Operator::BrIf { .. } => counts.br_if += 1,
                Operator::End => counts.end += 1,
                Operator::Call { .. } | Operator::CallIndirect { .. } => counts.call += 1,
                Operator::Return | Operator::ReturnCall { .. } => counts.return_ += 1,
                _ => {}
            }
        }
    }
    Ok(counts)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("r114_operator_census: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let corpus = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: r114_operator_census CORPUS OUTPUT")?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: r114_operator_census CORPUS OUTPUT")?;
    if arguments.next().is_some() {
        return Err("usage: r114_operator_census CORPUS OUTPUT".into());
    }

    let mut names = fs::read_dir(&corpus)
        .map_err(|error| format!("{}: {error}", corpus.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with("-structured.wasm"))
        .collect::<Vec<_>>();
    names.sort();
    if names.is_empty() {
        return Err("corpus has no control structured modules".into());
    }

    let mut report = String::from(
        "module\tside\tbytes\tfunctions\toperators\tlocal_get\tlocal_set\tlocal_tee\tif\tbr\tbr_if\tend\tcall\treturn\n",
    );
    let mut control_total = Counts::default();
    let mut candidate_total = Counts::default();
    for control_name in names {
        let stem = control_name
            .strip_suffix("-structured.wasm")
            .ok_or("invalid control suffix")?;
        let candidate_name = format!("{stem}-structured-lazy-pc.wasm");
        let control = census(&corpus.join(&control_name))?;
        let candidate = census(&corpus.join(&candidate_name))?;
        control_total.add(control);
        candidate_total.add(candidate);
        writeln!(report, "{stem}\tcontrol\t{}", control.fields()).unwrap();
        writeln!(report, "{stem}\tcandidate\t{}", candidate.fields()).unwrap();
    }
    writeln!(report, "TOTAL\tcontrol\t{}", control_total.fields()).unwrap();
    writeln!(report, "TOTAL\tcandidate\t{}", candidate_total.fields()).unwrap();
    fs::write(&output, report).map_err(|error| format!("{}: {error}", output.display()))?;
    eprintln!(
        "R114 operator census: {} paired modules, bytes {} -> {}, operators {} -> {}",
        control_total.functions,
        control_total.bytes,
        candidate_total.bytes,
        control_total.operators,
        candidate_total.operators,
    );
    Ok(())
}
