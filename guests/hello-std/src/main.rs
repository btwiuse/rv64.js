// Full-std guest: exercises musl startup (auxv, TLS, malloc via
// brk/mmap), println (writev), String formatting, env, and exit codes.

fn main() {
    println!("hello from Rust std on rv64.js!");

    let args: Vec<String> = std::env::args().collect();
    println!("args: {:?}", args);

    let mut v: Vec<u64> = (1..=10).collect();
    v.iter_mut().for_each(|x| *x *= *x);
    let sum: u64 = v.iter().sum();
    println!("sum of squares 1..10 = {sum}");

    let s = format!("{:>12}", "aligned");
    println!("fmt: [{s}]");

    let mut map = std::collections::HashMap::new();
    for w in "the quick brown fox jumps over the lazy dog the end".split(' ') {
        *map.entry(w).or_insert(0) += 1;
    }
    println!("'the' appears {} times", map["the"]);

    std::process::exit(if sum == 385 { 0 } else { 1 });
}
