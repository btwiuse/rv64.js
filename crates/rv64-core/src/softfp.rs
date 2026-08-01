//! Software floating point with exact IEEE 754 exception flags.
//!
//! A Rust port of TinyEMU's softfp (Copyright (c) 2016 Fabrice Bellard,
//! MIT license — see reference/tinyemu/softfp_template.h). Same internal
//! representation: mantissa normalized to bit F_SIZE-2 with RND_SIZE
//! guard/sticky bits, single round_pack step setting NX/UF/OF.

// fflags bits (RISC-V order)
pub const FFLAG_INEXACT: u32 = 1 << 0;
pub const FFLAG_UNDERFLOW: u32 = 1 << 1;
pub const FFLAG_DIVIDE_ZERO: u32 = 1 << 2;
pub const FFLAG_OVERFLOW: u32 = 1 << 3;
pub const FFLAG_INVALID_OP: u32 = 1 << 4;

// Rounding modes (RISC-V frm encoding)
pub const RM_RNE: u32 = 0;
pub const RM_RTZ: u32 = 1;
pub const RM_RDN: u32 = 2;
pub const RM_RUP: u32 = 3;
pub const RM_RMM: u32 = 4;

macro_rules! softfp {
    ($mod:ident, $u:ty, $ul:ty, $f_size:expr, $mant_size:expr, $exp_size:expr) => {
        pub mod $mod {
            use super::*;

            pub const F_SIZE: u32 = $f_size;
            pub const MANT_SIZE: u32 = $mant_size;
            pub const EXP_SIZE: u32 = $exp_size;
            pub const EXP_MASK: u32 = (1 << EXP_SIZE) - 1;
            pub const MANT_MASK: $u = (1 << MANT_SIZE) - 1;
            pub const SIGN_MASK: $u = 1 << (F_SIZE - 1);
            pub const IMANT_SIZE: u32 = F_SIZE - 2;
            pub const RND_SIZE: u32 = IMANT_SIZE - MANT_SIZE;
            pub const QNAN_MASK: $u = 1 << (MANT_SIZE - 1);
            pub const QNAN: $u = ((EXP_MASK as $u) << MANT_SIZE) | (1 << (MANT_SIZE - 1));

            #[inline]
            pub fn pack(sign: u32, exp: u32, mant: $u) -> $u {
                ((sign as $u) << (F_SIZE - 1)) | ((exp as $u) << MANT_SIZE) | (mant & MANT_MASK)
            }

            #[inline]
            fn rshift_rnd(a: $u, d: i32) -> $u {
                if d == 0 {
                    a
                } else if d >= F_SIZE as i32 {
                    (a != 0) as $u
                } else {
                    let mask: $u = (1 << d) - 1;
                    (a >> d) | ((a & mask) != 0) as $u
                }
            }

            /// mant has its MSB at bit F_SIZE-2 (RND_SIZE guard bits below).
            fn round_pack(sign: u32, mut exp: i32, mut mant: $u, rm: u32, fl: &mut u32) -> $u {
                let addend: $u = match rm {
                    RM_RNE | RM_RMM => 1 << (RND_SIZE - 1),
                    RM_RTZ => 0,
                    _ => {
                        if (sign ^ (rm & 1)) != 0 {
                            (1 << RND_SIZE) - 1
                        } else {
                            0
                        }
                    }
                };
                let rnd_bits: $u;
                if exp <= 0 {
                    // Underflow flag: rounded result subnormal and inexact.
                    let is_subnormal = exp < 0 || mant.wrapping_add(addend) < (1 << (F_SIZE - 1));
                    mant = rshift_rnd(mant, 1 - exp);
                    rnd_bits = mant & ((1 << RND_SIZE) - 1);
                    if is_subnormal && rnd_bits != 0 {
                        *fl |= FFLAG_UNDERFLOW;
                    }
                    exp = 1;
                } else {
                    rnd_bits = mant & ((1 << RND_SIZE) - 1);
                }
                if rnd_bits != 0 {
                    *fl |= FFLAG_INEXACT;
                }
                mant = mant.wrapping_add(addend) >> RND_SIZE;
                // halfway: round to even
                if rm == RM_RNE && rnd_bits == (1 << (RND_SIZE - 1)) {
                    mant &= !1;
                }
                exp += (mant >> (MANT_SIZE + 1)) as i32;
                if mant <= MANT_MASK {
                    exp = 0; // subnormal or zero
                } else if exp >= EXP_MASK as i32 {
                    if addend == 0 {
                        exp = EXP_MASK as i32 - 1;
                        mant = MANT_MASK;
                    } else {
                        exp = EXP_MASK as i32; // infinity
                        mant = 0;
                    }
                    *fl |= FFLAG_OVERFLOW | FFLAG_INEXACT;
                }
                pack(sign, exp as u32, mant)
            }

            /// mant has at most F_SIZE-1 bits.
            fn normalize(sign: u32, exp: i32, mant: $u, rm: u32, fl: &mut u32) -> $u {
                let shift = mant.leading_zeros() as i32 - (F_SIZE - 1 - IMANT_SIZE) as i32;
                round_pack(sign, exp - shift, mant << shift, rm, fl)
            }

            /// Double-word mantissa variant (hi has at most F_SIZE-1 bits).
            fn normalize2(
                sign: u32,
                exp: i32,
                mut hi: $u,
                mut lo: $u,
                rm: u32,
                fl: &mut u32,
            ) -> $u {
                let l = if hi == 0 {
                    F_SIZE as i32 + lo.leading_zeros() as i32
                } else {
                    hi.leading_zeros() as i32
                };
                let shift = l - (F_SIZE - 1 - IMANT_SIZE) as i32;
                let exp = exp - shift;
                if shift == 0 {
                    hi |= (lo != 0) as $u;
                } else if shift < F_SIZE as i32 {
                    hi = (hi << shift) | (lo >> (F_SIZE as i32 - shift));
                    lo <<= shift;
                    hi |= (lo != 0) as $u;
                } else {
                    hi = lo << (shift - F_SIZE as i32);
                }
                round_pack(sign, exp, hi, rm, fl)
            }

            #[inline]
            pub fn is_nan(a: $u) -> bool {
                ((a >> MANT_SIZE) as u32 & EXP_MASK) == EXP_MASK && (a & MANT_MASK) != 0
            }

            #[inline]
            pub fn is_signan(a: $u) -> bool {
                let exp1 = (a >> (MANT_SIZE - 1)) as u32 & ((1 << (EXP_SIZE + 1)) - 1);
                exp1 == 2 * EXP_MASK && (a & MANT_MASK) != 0
            }

            fn normalize_subnormal(exp: &mut i32, mant: $u) -> $u {
                let shift = MANT_SIZE as i32 - (F_SIZE as i32 - 1 - mant.leading_zeros() as i32);
                *exp = 1 - shift;
                mant << shift
            }

            pub fn add(mut a: $u, mut b: $u, rm: u32, fl: &mut u32) -> $u {
                // swap so |a| >= |b|
                if (a & !SIGN_MASK) < (b & !SIGN_MASK) {
                    core::mem::swap(&mut a, &mut b);
                }
                let mut a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                let mut a_exp = (a >> MANT_SIZE) as u32 & EXP_MASK;
                let mut b_exp = (b >> MANT_SIZE) as u32 & EXP_MASK;
                let mut a_mant = (a & MANT_MASK) << 3;
                let mut b_mant = (b & MANT_MASK) << 3;
                if a_exp == EXP_MASK {
                    return if a_mant != 0 {
                        if (a_mant & (QNAN_MASK << 3)) == 0 || is_signan(b) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        QNAN
                    } else if b_exp == EXP_MASK && a_sign != b_sign {
                        *fl |= FFLAG_INVALID_OP;
                        QNAN
                    } else {
                        a // infinity
                    };
                }
                if a_exp == 0 {
                    a_exp = 1;
                } else {
                    a_mant |= 1 << (MANT_SIZE + 3);
                }
                if b_exp == 0 {
                    b_exp = 1;
                } else {
                    b_mant |= 1 << (MANT_SIZE + 3);
                }
                b_mant = rshift_rnd(b_mant, (a_exp - b_exp) as i32);
                if a_sign == b_sign {
                    a_mant += b_mant;
                } else {
                    a_mant -= b_mant;
                    if a_mant == 0 {
                        a_sign = (rm == RM_RDN) as u32;
                    }
                }
                normalize(a_sign, a_exp as i32 + (RND_SIZE as i32 - 3), a_mant, rm, fl)
            }

            pub fn sub(a: $u, b: $u, rm: u32, fl: &mut u32) -> $u {
                add(a, b ^ SIGN_MASK, rm, fl)
            }

            #[inline]
            fn mul_u(a: $u, b: $u) -> ($u, $u) {
                let r = (a as $ul) * (b as $ul);
                ((r >> F_SIZE) as $u, r as $u)
            }

            pub fn mul(a: $u, b: $u, rm: u32, fl: &mut u32) -> $u {
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                let r_sign = a_sign ^ b_sign;
                let mut a_exp = (a >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut b_exp = (b >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut a_mant = a & MANT_MASK;
                let mut b_mant = b & MANT_MASK;
                if a_exp == EXP_MASK as i32 || b_exp == EXP_MASK as i32 {
                    return if is_nan(a) || is_nan(b) {
                        if is_signan(a) || is_signan(b) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        QNAN
                    } else if (a_exp == EXP_MASK as i32 && (b_exp == 0 && b_mant == 0))
                        || (b_exp == EXP_MASK as i32 && (a_exp == 0 && a_mant == 0))
                    {
                        *fl |= FFLAG_INVALID_OP;
                        QNAN
                    } else {
                        pack(r_sign, EXP_MASK, 0)
                    };
                }
                if a_exp == 0 {
                    if a_mant == 0 {
                        return pack(r_sign, 0, 0);
                    }
                    a_mant = normalize_subnormal(&mut a_exp, a_mant);
                } else {
                    a_mant |= 1 << MANT_SIZE;
                }
                if b_exp == 0 {
                    if b_mant == 0 {
                        return pack(r_sign, 0, 0);
                    }
                    b_mant = normalize_subnormal(&mut b_exp, b_mant);
                } else {
                    b_mant |= 1 << MANT_SIZE;
                }
                let r_exp = a_exp + b_exp - (1 << (EXP_SIZE - 1)) + 2;
                let (mut r_mant, r_low) = mul_u(a_mant << RND_SIZE, b_mant << (RND_SIZE + 1));
                r_mant |= (r_low != 0) as $u;
                normalize(r_sign, r_exp, r_mant, rm, fl)
            }

            pub fn fma(a: $u, b: $u, c: $u, rm: u32, fl: &mut u32) -> $u {
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                let c_sign = (c >> (F_SIZE - 1)) as u32;
                let mut r_sign = a_sign ^ b_sign;
                let mut a_exp = (a >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut b_exp = (b >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut c_exp = (c >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut a_mant = a & MANT_MASK;
                let mut b_mant = b & MANT_MASK;
                let mut c_mant = c & MANT_MASK;
                if a_exp == EXP_MASK as i32 || b_exp == EXP_MASK as i32 || c_exp == EXP_MASK as i32
                {
                    return if is_nan(a) || is_nan(b) || is_nan(c) {
                        if is_signan(a) || is_signan(b) || is_signan(c) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        QNAN
                    } else if (a_exp == EXP_MASK as i32 && (b_exp == 0 && b_mant == 0))
                        || (b_exp == EXP_MASK as i32 && (a_exp == 0 && a_mant == 0))
                        || ((a_exp == EXP_MASK as i32 || b_exp == EXP_MASK as i32)
                            && (c_exp == EXP_MASK as i32 && r_sign != c_sign))
                    {
                        *fl |= FFLAG_INVALID_OP;
                        QNAN
                    } else if c_exp == EXP_MASK as i32 {
                        pack(c_sign, EXP_MASK, 0)
                    } else {
                        pack(r_sign, EXP_MASK, 0)
                    };
                }
                // a * b == 0 cases
                let a_zero = a_exp == 0 && a_mant == 0;
                let b_zero = b_exp == 0 && b_mant == 0;
                if a_zero || b_zero {
                    return if c_exp == 0 && c_mant == 0 {
                        if c_sign != r_sign {
                            r_sign = (rm == RM_RDN) as u32;
                        }
                        pack(r_sign, 0, 0)
                    } else {
                        c
                    };
                }
                if a_exp == 0 {
                    a_mant = normalize_subnormal(&mut a_exp, a_mant);
                } else {
                    a_mant |= 1 << MANT_SIZE;
                }
                if b_exp == 0 {
                    b_mant = normalize_subnormal(&mut b_exp, b_mant);
                } else {
                    b_mant |= 1 << MANT_SIZE;
                }
                let mut r_exp = a_exp + b_exp - (1 << (EXP_SIZE - 1)) + 3;
                let (mut r1, mut r0) = mul_u(a_mant << RND_SIZE, b_mant << RND_SIZE);
                // normalize product to F_SIZE-3
                if r1 < (1 << (F_SIZE - 3)) {
                    r1 = (r1 << 1) | (r0 >> (F_SIZE - 1));
                    r0 <<= 1;
                    r_exp -= 1;
                }
                if c_exp == 0 {
                    if c_mant == 0 {
                        r1 |= (r0 != 0) as $u;
                        return normalize(r_sign, r_exp, r1, rm, fl);
                    }
                    c_mant = normalize_subnormal(&mut c_exp, c_mant);
                } else {
                    c_mant |= 1 << MANT_SIZE;
                }
                let mut c_exp = c_exp + 1;
                let mut c1: $u = c_mant << (RND_SIZE - 1);
                let mut c0: $u = 0;
                let mut c_sign = c_sign;
                let (mut r1, mut r0) = (r1, r0);
                // ensure |r| >= |c|
                if !(r_exp > c_exp || (r_exp == c_exp && r1 >= c1)) {
                    core::mem::swap(&mut r1, &mut c1);
                    core::mem::swap(&mut r0, &mut c0);
                    core::mem::swap(&mut r_exp, &mut c_exp);
                    core::mem::swap(&mut r_sign, &mut c_sign);
                }
                // shift c right by the exponent difference
                let shift = r_exp - c_exp;
                if shift >= 2 * F_SIZE as i32 {
                    c0 = ((c0 | c1) != 0) as $u;
                    c1 = 0;
                } else if shift > F_SIZE as i32 {
                    c0 = rshift_rnd(c1, shift - F_SIZE as i32);
                    c1 = 0;
                } else if shift == F_SIZE as i32 {
                    c0 = c1 | ((c0 != 0) as $u);
                    c1 = 0;
                } else if shift != 0 {
                    let mask: $u = (1 << shift) - 1;
                    c0 = (c1 << (F_SIZE as i32 - shift)) | (c0 >> shift) | ((c0 & mask) != 0) as $u;
                    c1 >>= shift;
                }
                if r_sign == c_sign {
                    r0 = r0.wrapping_add(c0);
                    r1 = r1.wrapping_add(c1).wrapping_add((r0 < c0) as $u);
                } else {
                    let tmp = r0;
                    r0 = r0.wrapping_sub(c0);
                    r1 = r1.wrapping_sub(c1).wrapping_sub((r0 > tmp) as $u);
                    if (r0 | r1) == 0 {
                        r_sign = (rm == RM_RDN) as u32;
                    }
                }
                normalize2(r_sign, r_exp, r1, r0, rm, fl)
            }

            fn divrem_u(ah: $u, al: $u, b: $u) -> ($u, $u) {
                let a = ((ah as $ul) << F_SIZE) | al as $ul;
                ((a / b as $ul) as $u, (a % b as $ul) as $u)
            }

            pub fn div(a: $u, b: $u, rm: u32, fl: &mut u32) -> $u {
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                let r_sign = a_sign ^ b_sign;
                let mut a_exp = (a >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut b_exp = (b >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut a_mant = a & MANT_MASK;
                let mut b_mant = b & MANT_MASK;
                if a_exp == EXP_MASK as i32 {
                    return if a_mant != 0 || is_nan(b) {
                        if is_signan(a) || is_signan(b) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        QNAN
                    } else if b_exp == EXP_MASK as i32 {
                        *fl |= FFLAG_INVALID_OP;
                        QNAN
                    } else {
                        pack(r_sign, EXP_MASK, 0)
                    };
                } else if b_exp == EXP_MASK as i32 {
                    return if b_mant != 0 {
                        if is_signan(a) || is_signan(b) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        QNAN
                    } else {
                        pack(r_sign, 0, 0)
                    };
                }
                if b_exp == 0 {
                    if b_mant == 0 {
                        return if a_exp == 0 && a_mant == 0 {
                            *fl |= FFLAG_INVALID_OP;
                            QNAN
                        } else {
                            *fl |= FFLAG_DIVIDE_ZERO;
                            pack(r_sign, EXP_MASK, 0)
                        };
                    }
                    b_mant = normalize_subnormal(&mut b_exp, b_mant);
                } else {
                    b_mant |= 1 << MANT_SIZE;
                }
                if a_exp == 0 {
                    if a_mant == 0 {
                        return pack(r_sign, 0, 0);
                    }
                    a_mant = normalize_subnormal(&mut a_exp, a_mant);
                } else {
                    a_mant |= 1 << MANT_SIZE;
                }
                let r_exp = a_exp - b_exp + (1 << (EXP_SIZE - 1)) - 1;
                let (mut r_mant, rem) = divrem_u(a_mant, 0, b_mant << 2);
                if rem != 0 {
                    r_mant |= 1;
                }
                normalize(r_sign, r_exp, r_mant, rm, fl)
            }

            /// sqrt(ah:al) with a < 2^(F_SIZE-2); returns (root, inexact).
            fn sqrtrem_u(ah: $u, al: $u) -> ($u, bool) {
                if ah == 0 && al == 0 {
                    return (0, false);
                }
                let a: $ul = ((ah as $ul) << F_SIZE) | al as $ul;
                let l: u32 = if ah != 0 {
                    2 * F_SIZE - (ah - 1).leading_zeros()
                } else {
                    F_SIZE - (al.wrapping_sub(1)).leading_zeros()
                };
                let mut u: $ul = 1 << ((l + 1) / 2);
                let mut s: $ul;
                loop {
                    s = u;
                    u = ((a / s) + s) / 2;
                    if u >= s {
                        break;
                    }
                }
                ((s as $u), a - s * s != 0)
            }

            pub fn sqrt(a: $u, rm: u32, fl: &mut u32) -> $u {
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let mut a_exp = (a >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                let mut a_mant = a & MANT_MASK;
                if a_exp == EXP_MASK as i32 {
                    if a_mant != 0 {
                        if is_signan(a) {
                            *fl |= FFLAG_INVALID_OP;
                        }
                        return QNAN;
                    } else if a_sign != 0 {
                        *fl |= FFLAG_INVALID_OP;
                        return QNAN;
                    } else {
                        return a; // +inf
                    }
                }
                if a_sign != 0 {
                    if a_exp == 0 && a_mant == 0 {
                        return a; // -0
                    }
                    *fl |= FFLAG_INVALID_OP;
                    return QNAN;
                }
                if a_exp == 0 {
                    if a_mant == 0 {
                        return pack(0, 0, 0);
                    }
                    a_mant = normalize_subnormal(&mut a_exp, a_mant);
                } else {
                    a_mant |= 1 << MANT_SIZE;
                }
                a_exp -= EXP_MASK as i32 / 2;
                if a_exp & 1 != 0 {
                    a_exp -= 1;
                    a_mant <<= 1;
                }
                a_exp = (a_exp >> 1) + EXP_MASK as i32 / 2;
                a_mant <<= F_SIZE - 4 - MANT_SIZE;
                let (mut root, inexact) = sqrtrem_u(a_mant, 0);
                if inexact {
                    root |= 1;
                }
                normalize(a_sign, a_exp, root, rm, fl)
            }

            /// RISC-V (IEEE 754-201x minimumNumber/maximumNumber) min/max.
            fn min_max_nan(a: $u, b: $u, fl: &mut u32) -> $u {
                if is_signan(a) || is_signan(b) {
                    *fl |= FFLAG_INVALID_OP;
                }
                if is_nan(a) {
                    if is_nan(b) {
                        QNAN
                    } else {
                        b
                    }
                } else {
                    a
                }
            }

            pub fn min(a: $u, b: $u, fl: &mut u32) -> $u {
                if is_nan(a) || is_nan(b) {
                    return min_max_nan(a, b, fl);
                }
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                if a_sign != b_sign {
                    if a_sign != 0 {
                        a
                    } else {
                        b
                    }
                } else if ((a < b) as u32 ^ a_sign) != 0 {
                    a
                } else {
                    b
                }
            }

            pub fn max(a: $u, b: $u, fl: &mut u32) -> $u {
                if is_nan(a) || is_nan(b) {
                    return min_max_nan(a, b, fl);
                }
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let b_sign = (b >> (F_SIZE - 1)) as u32;
                if a_sign != b_sign {
                    if a_sign != 0 {
                        b
                    } else {
                        a
                    }
                } else if ((a < b) as u32 ^ a_sign) != 0 {
                    b
                } else {
                    a
                }
            }

            pub fn eq_quiet(a: $u, b: $u, fl: &mut u32) -> bool {
                if is_nan(a) || is_nan(b) {
                    if is_signan(a) || is_signan(b) {
                        *fl |= FFLAG_INVALID_OP;
                    }
                    return false;
                }
                if ((a | b) << 1) == 0 {
                    return true; // ±0 == ∓0
                }
                a == b
            }

            pub fn le(a: $u, b: $u, fl: &mut u32) -> bool {
                if is_nan(a) || is_nan(b) {
                    *fl |= FFLAG_INVALID_OP;
                    return false;
                }
                let a_sign = a >> (F_SIZE - 1);
                let b_sign = b >> (F_SIZE - 1);
                if a_sign != b_sign {
                    a_sign != 0 || ((a | b) << 1) == 0
                } else if a_sign != 0 {
                    a >= b
                } else {
                    a <= b
                }
            }

            pub fn lt(a: $u, b: $u, fl: &mut u32) -> bool {
                if is_nan(a) || is_nan(b) {
                    *fl |= FFLAG_INVALID_OP;
                    return false;
                }
                let a_sign = a >> (F_SIZE - 1);
                let b_sign = b >> (F_SIZE - 1);
                if a_sign != b_sign {
                    a_sign != 0 && ((a | b) << 1) != 0
                } else if a_sign != 0 {
                    a > b
                } else {
                    a < b
                }
            }

            pub fn fclass(a: $u) -> u32 {
                let a_sign = (a >> (F_SIZE - 1)) as u32;
                let a_exp = (a >> MANT_SIZE) as u32 & EXP_MASK;
                let a_mant = a & MANT_MASK;
                if a_exp == EXP_MASK {
                    if a_mant != 0 {
                        if a_mant & QNAN_MASK != 0 {
                            1 << 9 // qNaN
                        } else {
                            1 << 8 // sNaN
                        }
                    } else if a_sign != 0 {
                        1 << 0
                    } else {
                        1 << 7
                    }
                } else if a_exp == 0 {
                    if a_mant == 0 {
                        if a_sign != 0 {
                            1 << 3
                        } else {
                            1 << 4
                        }
                    } else if a_sign != 0 {
                        1 << 2
                    } else {
                        1 << 5
                    }
                } else if a_sign != 0 {
                    1 << 1
                } else {
                    1 << 6
                }
            }

            // ---- float <-> integer (port of softfp_template_icvt.h) ----

            macro_rules! cvt_f_to_i {
                ($name:ident, $iu:ty, $isz:expr) => {
                    /// Convert to integer; `unsigned` selects the u-variant.
                    pub fn $name(a: $u, rm: u32, fl: &mut u32, unsigned: bool) -> $iu {
                        let mut a_sign = (a >> (F_SIZE - 1)) as u32;
                        let mut a_exp = (a >> MANT_SIZE) as u32 as i32 & EXP_MASK as i32;
                        let mut a_mant = a & MANT_MASK;
                        if a_exp == EXP_MASK as i32 && a_mant != 0 {
                            a_sign = 0; // NaN behaves like +inf
                        }
                        if a_exp == 0 {
                            a_exp = 1;
                        } else {
                            a_mant |= 1 << MANT_SIZE;
                        }
                        a_mant <<= RND_SIZE;
                        a_exp = a_exp - (EXP_MASK as i32 / 2) - MANT_SIZE as i32;

                        let r_max: $iu = if unsigned {
                            (a_sign as $iu).wrapping_sub(1)
                        } else {
                            (1 << ($isz - 1)) - ((a_sign ^ 1) as $iu)
                        };
                        let mut r: $iu;
                        if a_exp >= 0 {
                            if a_exp <= ($isz as i32 - 1 - MANT_SIZE as i32) {
                                r = ((a_mant >> RND_SIZE) as $iu) << a_exp;
                                if r > r_max {
                                    *fl |= FFLAG_INVALID_OP;
                                    return r_max;
                                }
                            } else {
                                *fl |= FFLAG_INVALID_OP;
                                return r_max;
                            }
                        } else {
                            a_mant = rshift_rnd(a_mant, -a_exp);
                            let addend: $u = match rm {
                                RM_RNE | RM_RMM => 1 << (RND_SIZE - 1),
                                RM_RTZ => 0,
                                _ => {
                                    if (a_sign ^ (rm & 1)) != 0 {
                                        (1 << RND_SIZE) - 1
                                    } else {
                                        0
                                    }
                                }
                            };
                            let rnd_bits = a_mant & ((1 << RND_SIZE) - 1);
                            a_mant = a_mant.wrapping_add(addend) >> RND_SIZE;
                            if rm == RM_RNE && rnd_bits == (1 << (RND_SIZE - 1)) {
                                a_mant &= !1;
                            }
                            if a_mant as u128 > r_max as u128 {
                                *fl |= FFLAG_INVALID_OP;
                                return r_max;
                            }
                            r = a_mant as $iu;
                            if rnd_bits != 0 {
                                *fl |= FFLAG_INEXACT;
                            }
                        }
                        if a_sign != 0 {
                            r = r.wrapping_neg();
                        }
                        r
                    }
                };
            }

            cvt_f_to_i!(cvt_to_i32, u32, 32);
            cvt_f_to_i!(cvt_to_i64, u64, 64);

            macro_rules! cvt_i_to_f {
                ($name:ident, $iu:ty, $is:ty, $isz:expr) => {
                    pub fn $name(a: $iu, rm: u32, fl: &mut u32, unsigned: bool) -> $u {
                        let a_sign: u32;
                        let mut r: $iu;
                        if !unsigned && (a as $is) < 0 {
                            a_sign = 1;
                            r = a.wrapping_neg();
                        } else {
                            a_sign = 0;
                            r = a;
                        }
                        let mut a_exp = (EXP_MASK as i32 / 2) + F_SIZE as i32 - 2;
                        // reduce range before generic normalization
                        let l = $isz as i32
                            - (if r == 0 { $isz } else { r.leading_zeros() }) as i32
                            - (F_SIZE as i32 - 1);
                        if l > 0 {
                            let mask: $iu = ((1 as $iu) << l) - 1;
                            r = (r >> l) | (((r & mask) != 0) as $iu);
                            a_exp += l;
                        }
                        normalize(a_sign, a_exp, r as $u, rm, fl)
                    }
                };
            }

            cvt_i_to_f!(cvt_from_i32, u32, i32, 32u32);
            cvt_i_to_f!(cvt_from_i64, u64, i64, 64u32);
        }
    };
}

softfp!(sf32, u32, u64, 32, 23, 8);
softfp!(sf64, u64, u128, 64, 52, 11);

// ---- conversions between f32 and f64 --------------------------------------

/// f32 -> f64: exact (except sNaN -> NV + canonical qNaN).
pub fn cvt_sf32_sf64(a: u32, fl: &mut u32) -> u64 {
    let a_sign = (a >> 31) as u64;
    let a_exp = (a >> 23) & 0xff;
    let a_mant = a & 0x7f_ffff;
    if a_exp == 0xff {
        if a_mant != 0 {
            if sf32::is_signan(a) {
                *fl |= FFLAG_INVALID_OP;
            }
            return sf64::QNAN;
        }
        return (a_sign << 63) | (0x7ff << 52);
    }
    if a_exp == 0 {
        if a_mant == 0 {
            return a_sign << 63;
        }
        // normalize subnormal
        let shift = 23 - (31 - a_mant.leading_zeros());
        let exp = 1 - shift as i32;
        let mant = (a_mant << shift) & 0x7f_ffff;
        let e = (exp - 0x7f + 0x3ff) as u64;
        return (a_sign << 63) | (e << 52) | ((mant as u64) << (52 - 23));
    }
    let e = (a_exp - 0x7f + 0x3ff) as u64;
    (a_sign << 63) | (e << 52) | ((a_mant as u64) << (52 - 23))
}

/// f64 -> f32: rounded.
pub fn cvt_sf64_sf32(a: u64, rm: u32, fl: &mut u32) -> u32 {
    let a_sign = (a >> 63) as u32;
    let mut a_exp = ((a >> 52) & 0x7ff) as i32;
    let mut a_mant = a & 0xf_ffff_ffff_ffff;
    if a_exp == 0x7ff {
        if a_mant != 0 {
            if sf64::is_signan(a) {
                *fl |= FFLAG_INVALID_OP;
            }
            return sf32::QNAN;
        }
        return sf32::pack(a_sign, 0xff, 0);
    }
    if a_exp == 0 {
        if a_mant == 0 {
            return sf32::pack(a_sign, 0, 0);
        }
        // Subnormal: shifting the leading 1 up to bit 52 makes it act as
        // the hidden bit (same as the template's normalize_subnormal).
        let shift = 52 - (63 - a_mant.leading_zeros());
        a_exp = 1 - shift as i32;
        a_mant <<= shift;
    } else {
        a_mant |= 1 << 52;
    }
    let exp32 = a_exp - 0x3ff + 0x7f;
    let mant = rshift_rnd_u64(a_mant, 52 - 30);
    normalize_sf32_from(a_sign, exp32, mant as u32, rm, fl)
}

fn rshift_rnd_u64(a: u64, d: i32) -> u64 {
    if d == 0 {
        a
    } else if d >= 64 {
        (a != 0) as u64
    } else {
        let mask = (1u64 << d) - 1;
        (a >> d) | ((a & mask) != 0) as u64
    }
}

fn normalize_sf32_from(sign: u32, exp: i32, mant: u32, rm: u32, fl: &mut u32) -> u32 {
    // mant has at most 31 bits; reuse sf32's normalize path via a shim:
    // replicate normalize() since it's private — same algorithm.
    let shift = mant.leading_zeros() as i32 - 1; // to bit 30
    round_pack_sf32(sign, exp - shift, mant << shift, rm, fl)
}

fn round_pack_sf32(sign: u32, exp: i32, mant: u32, rm: u32, fl: &mut u32) -> u32 {
    // Delegate to sf32 by reconstructing through its public API:
    // add(x, +0) never rounds; so instead expose a tiny local copy of
    // round_pack for f32 (kept in sync with the macro version).
    const RND_SIZE: u32 = 7;
    const MANT_SIZE: u32 = 23;
    const EXP_MASK: i32 = 0xff;
    const MANT_MASK: u32 = (1 << MANT_SIZE) - 1;
    let mut exp = exp;
    let mut mant = mant;
    let addend: u32 = match rm {
        RM_RNE | RM_RMM => 1 << (RND_SIZE - 1),
        RM_RTZ => 0,
        _ => {
            if (sign ^ (rm & 1)) != 0 {
                (1 << RND_SIZE) - 1
            } else {
                0
            }
        }
    };
    let rnd_bits: u32;
    if exp <= 0 {
        let is_subnormal = exp < 0 || mant.wrapping_add(addend) < (1 << 31);
        mant = rshift_rnd_u64(mant as u64, 1 - exp) as u32;
        rnd_bits = mant & ((1 << RND_SIZE) - 1);
        if is_subnormal && rnd_bits != 0 {
            *fl |= FFLAG_UNDERFLOW;
        }
        exp = 1;
    } else {
        rnd_bits = mant & ((1 << RND_SIZE) - 1);
    }
    if rnd_bits != 0 {
        *fl |= FFLAG_INEXACT;
    }
    mant = mant.wrapping_add(addend) >> RND_SIZE;
    if rm == RM_RNE && rnd_bits == (1 << (RND_SIZE - 1)) {
        mant &= !1;
    }
    exp += (mant >> (MANT_SIZE + 1)) as i32;
    if mant <= MANT_MASK {
        exp = 0;
    } else if exp >= EXP_MASK {
        if addend == 0 {
            exp = EXP_MASK - 1;
            mant = MANT_MASK;
        } else {
            exp = EXP_MASK;
            mant = 0;
        }
        *fl |= FFLAG_OVERFLOW | FFLAG_INEXACT;
    }
    sf32::pack(sign, exp as u32, mant)
}
