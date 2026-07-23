// FP validation guest: exercises F/D arithmetic, conversions, comparisons,
// classification, min/max, FMA, and float formatting (ryu). Prints PASS/FAIL
// per check; the host test asserts on exit code.

static mut FAILS: u32 = 0;

fn check(name: &str, ok: bool) {
    if !ok {
        unsafe { FAILS += 1 };
        println!("FAIL: {name}");
    } else {
        println!("ok: {name}");
    }
}

#[inline(never)]
fn opaque_f64(x: f64) -> f64 {
    std::hint::black_box(x)
}

#[inline(never)]
fn opaque_f32(x: f32) -> f32 {
    std::hint::black_box(x)
}

fn main() {
    // basic arithmetic
    let a = opaque_f64(0.1);
    let b = opaque_f64(0.2);
    check("f64 add", a + b == 0.30000000000000004);
    check("f64 mul", opaque_f64(1e308) * 10.0 == f64::INFINITY);
    check("f64 div0", opaque_f64(1.0) / 0.0 == f64::INFINITY);
    check("f64 sqrt", opaque_f64(2.0).sqrt() == 1.4142135623730951);
    check("f32 sub", opaque_f32(1.5f32) - 0.25 == 1.25);

    // NaN behavior
    let nan = opaque_f64(f64::NAN);
    check("nan != nan", nan != nan);
    check("nan min", nan.min(1.0) == 1.0); // Rust min = IEEE minNum ≈ RISC-V fmin
    check("-0 vs +0 fmin", (-0.0f64).min(0.0).is_sign_negative());

    // conversions incl. RISC-V saturation semantics
    check("f2i", opaque_f64(3.99) as i64 == 3);
    check("f2i neg", opaque_f64(-3.99) as i64 == -3);
    check("f2i sat hi", opaque_f64(1e300) as i64 == i64::MAX);
    check("f2i sat lo", opaque_f64(-1e300) as i64 == i64::MIN);
    check("u2f", (u64::MAX as f64) == 1.8446744073709552e19);
    check("i2f32", (16_777_217i64 as f32) == 16_777_216.0f32); // rounds
    check("f64->f32", (opaque_f64(1e50) as f32) == f32::INFINITY);

    // classification
    check("classify sub", f64::MIN_POSITIVE / 2.0 != 0.0); // subnormal survives
    check("inf class", f32::INFINITY.is_infinite());

    // FMA (mul_add lowers to fmadd): (2^27+1)(2^27-1) = 2^54-1, which rounds
    // to 2^54 in f64 — so the fused residual is exactly -1.0.
    let x = opaque_f64(134_217_729.0); // 2^27 + 1
    let y = opaque_f64(134_217_727.0); // 2^27 - 1
    let z = x * y;
    check("fma fused", x.mul_add(y, -z) == -1.0);

    // formatting round-trip (ryu exercises lots of FP+int ops)
    check("fmt f64", format!("{}", 0.3) == "0.3");
    check("fmt exp", format!("{:e}", 1234.5) == "1.2345e3");
    let parsed: f64 = "2.718281828459045".parse().unwrap();
    check("parse f64", parsed == std::f64::consts::E);

    let fails = unsafe { FAILS };
    println!("--- {} failures", fails);
    std::process::exit(if fails == 0 { 0 } else { 1 });
}
