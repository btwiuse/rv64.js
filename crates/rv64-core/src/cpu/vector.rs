//! Ratified RISC-V Vector 1.0 interpreter support.
//!
//! The architectural baseline is RV64GCV with VLEN=128 and ELEN=64.  Vector
//! instructions deliberately remain on the authoritative interpreter path;
//! the JIT has no RVV lowering in this phase.

use super::*;

pub(super) const VLEN_BITS: usize = 128;
pub(super) const VLEN_BYTES: usize = VLEN_BITS / 8;
const VREG_COUNT: usize = 32;
const VILL: u64 = 1 << 63;

/// Complete architectural vector-register and vector-CSR state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorState {
    /// v0..v31, each VLEN bits, in architectural little-endian byte order.
    pub regs: [[u8; VLEN_BYTES]; VREG_COUNT],
    pub vl: u64,
    pub vtype: u64,
    pub vstart: u64,
    pub vxrm: u8,
    pub vxsat: bool,
    decoded: Option<VectorConfig>,
}

impl Default for VectorState {
    fn default() -> Self {
        Self {
            regs: [[0; VLEN_BYTES]; VREG_COUNT],
            vl: 0,
            // The vector extension resets into an illegal configuration. A
            // vset* instruction must establish vtype/vl before ordinary ops.
            vtype: VILL,
            vstart: 0,
            vxrm: 0,
            vxsat: false,
            decoded: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorConfig {
    sew: usize,
    lmul_num: usize,
    lmul_den: usize,
    lmul_exp: i8,
    vlmax: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GroupRatio {
    num: usize,
    den: usize,
}

const MASK_REGISTER_RATIO: GroupRatio = GroupRatio { num: 1, den: 1 };

impl GroupRatio {
    #[inline(always)]
    fn register_span(self) -> usize {
        if self.num < self.den {
            1
        } else {
            debug_assert_eq!(self.num % self.den, 0);
            self.num / self.den
        }
    }
}

impl VectorConfig {
    fn decode(raw: u64) -> Option<Self> {
        // In the ratified V 1.0 format, bits XLEN-2:8 are reserved and must
        // be zero. Bit XLEN-1 is VILL.
        if raw & VILL != 0 || raw >> 8 != 0 {
            return None;
        }

        let lmul_exp = match (raw & 7) as u8 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            5 => -3,
            6 => -2,
            7 => -1,
            // 100 is the reserved LMUL encoding.
            _ => return None,
        };
        let vsew = ((raw >> 3) & 7) as usize;
        if vsew > 3 {
            return None;
        }
        let sew_log = 3 + vsew as i32;
        // ELEN=64, and fractional LMUL requires SEW <= LMUL*ELEN.
        if sew_log > 6 + i32::from(lmul_exp) {
            return None;
        }
        let vlmax_log = 7 + i32::from(lmul_exp) - sew_log;
        if vlmax_log < 0 {
            return None;
        }
        let (lmul_num, lmul_den) = if lmul_exp >= 0 {
            (1usize << lmul_exp, 1)
        } else {
            (1, 1usize << -lmul_exp)
        };
        Some(Self {
            sew: 1usize << sew_log,
            lmul_num,
            lmul_den,
            lmul_exp,
            vlmax: 1usize << vlmax_log,
        })
    }

    #[inline(always)]
    fn lmul(self) -> GroupRatio {
        GroupRatio {
            num: self.lmul_num,
            den: self.lmul_den,
        }
    }

    #[inline(always)]
    fn emul(self, eew: usize) -> Option<GroupRatio> {
        let eew_log = match eew {
            8 => 3,
            16 => 4,
            32 => 5,
            64 => 6,
            _ => return None,
        };
        let sew_log = match self.sew {
            8 => 3,
            16 => 4,
            32 => 5,
            64 => 6,
            _ => unreachable!("decoded SEW"),
        };
        let exponent = i32::from(self.lmul_exp) + eew_log - sew_log;
        if !(-3..=3).contains(&exponent) {
            return None;
        }
        Some(if exponent >= 0 {
            GroupRatio {
                num: 1usize << exponent,
                den: 1,
            }
        } else {
            GroupRatio {
                num: 1,
                den: 1usize << -exponent,
            }
        })
    }

    #[inline]
    fn emul_register_span(self, eew: usize) -> Option<usize> {
        let eew_log = match eew {
            8 => 3,
            16 => 4,
            32 => 5,
            64 => 6,
            _ => return None,
        };
        let sew_log = match self.sew {
            8 => 3,
            16 => 4,
            32 => 5,
            64 => 6,
            _ => unreachable!("decoded SEW"),
        };
        let exponent = i32::from(self.lmul_exp) + eew_log - sew_log;
        (-3..=3)
            .contains(&exponent)
            .then(|| if exponent > 0 { 1usize << exponent } else { 1 })
    }
}

impl VectorState {
    #[inline(always)]
    fn config(&self) -> Option<VectorConfig> {
        self.decoded
    }

    fn install_config(
        &mut self,
        raw_type: u64,
        rd: usize,
        rs1_is_zero: bool,
        avl: u64,
        immediate_avl: bool,
    ) -> u64 {
        let old_vlmax = self.config().map(|c| c.vlmax);
        let new_config = VectorConfig::decode(raw_type);
        let retain_vl = !immediate_avl && rd == 0 && rs1_is_zero;
        let legal = new_config.filter(|new| !retain_vl || old_vlmax == Some(new.vlmax));

        if let Some(config) = legal {
            self.vtype = raw_type & 0xff;
            self.decoded = Some(config);
            self.vl = if immediate_avl {
                avl.min(config.vlmax as u64)
            } else if retain_vl {
                self.vl
            } else if rs1_is_zero {
                config.vlmax as u64
            } else {
                avl.min(config.vlmax as u64)
            };
        } else {
            self.vtype = VILL;
            self.vl = 0;
            self.decoded = None;
        }
        self.vstart = 0;
        self.vl
    }

    #[inline]
    fn mask_bit(&self, index: usize) -> bool {
        self.regs[0][index >> 3] & (1 << (index & 7)) != 0
    }

    #[inline]
    fn set_flat_byte(&mut self, offset: usize, value: u8) {
        self.regs[offset / VLEN_BYTES][offset % VLEN_BYTES] = value;
    }

    fn read_element(&self, reg: usize, index: usize, eew: usize) -> u64 {
        let bytes = eew / 8;
        let offset = reg * VLEN_BYTES + index * bytes;
        debug_assert!(offset + bytes <= VREG_COUNT * VLEN_BYTES);
        let address = unsafe { self.regs.as_ptr().cast::<u8>().add(offset) };
        unsafe {
            // SAFETY: the validated group and element index bound the access
            // within the contiguous architectural register file. Vector
            // elements are little-endian and may be naturally unaligned when
            // viewed through a flattened register group.
            match eew {
                8 => core::ptr::read(address) as u64,
                16 => u16::from_le(core::ptr::read_unaligned(address.cast::<u16>())) as u64,
                32 => u32::from_le(core::ptr::read_unaligned(address.cast::<u32>())) as u64,
                64 => u64::from_le(core::ptr::read_unaligned(address.cast::<u64>())),
                _ => unreachable!("validated vector EEW"),
            }
        }
    }

    fn write_element(&mut self, reg: usize, index: usize, eew: usize, value: u64) {
        let bytes = eew / 8;
        let offset = reg * VLEN_BYTES + index * bytes;
        debug_assert!(offset + bytes <= VREG_COUNT * VLEN_BYTES);
        let address = unsafe { self.regs.as_mut_ptr().cast::<u8>().add(offset) };
        unsafe {
            // SAFETY: identical bounds argument to `read_element`; unaligned
            // stores preserve the architectural byte layout across registers.
            match eew {
                8 => core::ptr::write(address, value as u8),
                16 => core::ptr::write_unaligned(address.cast::<u16>(), (value as u16).to_le()),
                32 => core::ptr::write_unaligned(address.cast::<u32>(), (value as u32).to_le()),
                64 => core::ptr::write_unaligned(address.cast::<u64>(), value.to_le()),
                _ => unreachable!("validated vector EEW"),
            }
        }
    }
}

#[inline]
fn read_single_register_element(bytes: &[u8; VLEN_BYTES], index: usize, eew: usize) -> u64 {
    let width = eew / 8;
    let offset = index * width;
    debug_assert!(offset + width <= VLEN_BYTES);
    let address = unsafe { bytes.as_ptr().add(offset) };
    unsafe {
        // SAFETY: callers use this only for LMUL groups contained in one
        // register, and validate the element against that configuration's
        // VLMAX before reading.
        match eew {
            8 => core::ptr::read(address) as u64,
            16 => u16::from_le(core::ptr::read_unaligned(address.cast::<u16>())) as u64,
            32 => u32::from_le(core::ptr::read_unaligned(address.cast::<u32>())) as u64,
            64 => u64::from_le(core::ptr::read_unaligned(address.cast::<u64>())),
            _ => unreachable!("validated vector EEW"),
        }
    }
}

fn validate_group(base: usize, ratio: GroupRatio, fields: usize) -> bool {
    let span = ratio.register_span();
    fields != 0
        && fields * ratio.num <= 8 * ratio.den
        && (span == 1 || base % span == 0)
        && base
            .checked_add(span * fields)
            .is_some_and(|end| end <= VREG_COUNT)
}

fn groups_overlap(a: usize, ar: GroupRatio, an: usize, b: usize, br: GroupRatio) -> bool {
    let a_end = a + ar.register_span() * an;
    let b_end = b + br.register_span();
    a < b_end && b < a_end
}

/// Ratified V permits mixed-width destination/source overlap only in the
/// direction that preserves restartability: narrowing at the low end, or
/// widening at the high end when the narrow source occupies at least one
/// complete register. Equal-width operands can overlap freely.
fn destination_source_overlap_legal(
    destination: usize,
    destination_ratio: GroupRatio,
    destination_eew: usize,
    source: usize,
    source_ratio: GroupRatio,
    source_eew: usize,
) -> bool {
    if !groups_overlap(destination, destination_ratio, 1, source, source_ratio) {
        return true;
    }
    if destination_eew == source_eew {
        return true;
    }
    if destination_eew < source_eew {
        return destination == source;
    }
    source_ratio.num >= source_ratio.den
        && destination + destination_ratio.register_span() == source + source_ratio.register_span()
}

/// A vector register cannot supply two source operands with different EEWs in
/// one instruction. The implicit v0 predicate is a one-bit source operand.
fn mixed_width_sources_overlap(
    first: usize,
    first_ratio: GroupRatio,
    first_eew: usize,
    second: usize,
    second_ratio: GroupRatio,
    second_eew: usize,
) -> bool {
    first_eew != second_eew && groups_overlap(first, first_ratio, 1, second, second_ratio)
}

#[inline]
fn element_mask(bits: usize) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

#[inline]
fn signed_element(value: u64, bits: usize) -> i64 {
    if bits == 64 {
        value as i64
    } else {
        ((value << (64 - bits)) as i64) >> (64 - bits)
    }
}

#[inline]
fn simm5(field: usize) -> u64 {
    (((field as u32) << 27) as i32 >> 27) as i64 as u64
}

/// Return the increment prescribed by `vxrm` when discarding `shift` low
/// bits.  The input is the two's-complement bit pattern of either an unsigned
/// or signed intermediate, so the same rule applies to both kinds of fixed-
/// point instruction.
#[inline]
fn fixed_round_increment(value: u128, shift: usize, vxrm: u8) -> u128 {
    if shift == 0 {
        return 0;
    }
    let retained_lsb = (value >> shift) & 1;
    let highest_discarded = (value >> (shift - 1)) & 1;
    let discarded_mask = (1u128 << shift) - 1;
    let discarded = value & discarded_mask;
    match vxrm & 3 {
        0 => highest_discarded, // round-to-nearest-up
        1 => {
            highest_discarded & (((discarded & (discarded_mask >> 1)) != 0) as u128 | retained_lsb)
        }
        2 => 0,                                               // truncate
        3 => ((retained_lsb == 0) && discarded != 0) as u128, // round-to-odd
        _ => unreachable!(),
    }
}

#[inline]
fn fixed_round_unsigned(value: u128, shift: usize, vxrm: u8) -> u128 {
    (value >> shift).wrapping_add(fixed_round_increment(value, shift, vxrm))
}

#[inline]
fn fixed_round_signed(value: i128, shift: usize, vxrm: u8) -> i128 {
    (value >> shift).wrapping_add(fixed_round_increment(value as u128, shift, vxrm) as i128)
}

#[inline]
fn signed_narrow(value: u64, bits: usize) -> i128 {
    signed_element(value, bits) as i128
}

// Tables and bit-exact algorithms from the ratified RVV 1.0 reciprocal-
// estimate definitions. They are shared by f32 and f64; G does not imply the
// optional vector half-precision extensions.
const FRSQRT7_TABLE: [u8; 128] = [
    52, 51, 50, 48, 47, 46, 44, 43, 42, 41, 40, 39, 38, 36, 35, 34, 33, 32, 31, 30, 30, 29, 28, 27,
    26, 25, 24, 23, 23, 22, 21, 20, 19, 19, 18, 17, 16, 16, 15, 14, 14, 13, 12, 12, 11, 10, 10, 9,
    9, 8, 7, 7, 6, 6, 5, 4, 4, 3, 3, 2, 2, 1, 1, 0, 127, 125, 123, 121, 119, 118, 116, 114, 113,
    111, 109, 108, 106, 105, 103, 102, 100, 99, 97, 96, 95, 93, 92, 91, 90, 88, 87, 86, 85, 84, 83,
    82, 80, 79, 78, 77, 76, 75, 74, 73, 72, 71, 70, 70, 69, 68, 67, 66, 65, 64, 63, 63, 62, 61, 60,
    59, 59, 58, 57, 56, 56, 55, 54, 53,
];

const FREC7_TABLE: [u8; 128] = [
    127, 125, 123, 121, 119, 117, 116, 114, 112, 110, 109, 107, 105, 104, 102, 100, 99, 97, 96, 94,
    93, 91, 90, 88, 87, 85, 84, 83, 81, 80, 79, 77, 76, 75, 74, 72, 71, 70, 69, 68, 66, 65, 64, 63,
    62, 61, 60, 59, 58, 57, 56, 55, 54, 53, 52, 51, 50, 49, 48, 47, 46, 45, 44, 43, 42, 41, 40, 40,
    39, 38, 37, 36, 35, 35, 34, 33, 32, 31, 31, 30, 29, 28, 28, 27, 26, 25, 25, 24, 23, 23, 22, 21,
    21, 20, 19, 19, 18, 17, 17, 16, 15, 15, 14, 14, 13, 12, 12, 11, 11, 10, 9, 9, 8, 8, 7, 7, 6, 5,
    5, 4, 4, 3, 3, 2, 2, 1, 1, 0,
];

fn fp_estimate_parts(value: u64, exp_bits: usize, frac_bits: usize) -> (u64, u64, u64) {
    let frac_mask = (1u64 << frac_bits) - 1;
    let exp_mask = (1u64 << exp_bits) - 1;
    (
        value >> (frac_bits + exp_bits),
        (value >> frac_bits) & exp_mask,
        value & frac_mask,
    )
}

fn rvv_frsqrt7(value: u64, exp_bits: usize, frac_bits: usize, flags: &mut u32) -> u64 {
    use crate::softfp::{FFLAG_DIVIDE_ZERO, FFLAG_INVALID_OP};
    let (sign, mut exponent, mut fraction) = fp_estimate_parts(value, exp_bits, frac_bits);
    let exp_mask = (1u64 << exp_bits) - 1;
    let frac_mask = (1u64 << frac_bits) - 1;
    let quiet_bit = 1u64 << (frac_bits - 1);
    let infinity = exp_mask << frac_bits;
    let canonical_nan = infinity | quiet_bit;
    if exponent == exp_mask && fraction != 0 {
        if fraction & quiet_bit == 0 {
            *flags |= FFLAG_INVALID_OP;
        }
        return canonical_nan;
    }
    if sign != 0 && (exponent != 0 || fraction != 0) {
        *flags |= FFLAG_INVALID_OP;
        return canonical_nan;
    }
    if exponent == 0 && fraction == 0 {
        *flags |= FFLAG_DIVIDE_ZERO;
        return (sign << (exp_bits + frac_bits)) | infinity;
    }
    if exponent == exp_mask {
        return 0;
    }
    if exponent == 0 {
        while fraction >> (frac_bits - 1) == 0 {
            exponent = exponent.wrapping_sub(1);
            fraction <<= 1;
        }
        fraction = (fraction << 1) & frac_mask;
    }
    let index = (((exponent & 1) << 6) | (fraction >> (frac_bits - 6))) as usize;
    let output_fraction = (FRSQRT7_TABLE[index] as u64) << (frac_bits - 7);
    let half_exp_mask = (1u64 << (exp_bits - 1)) - 1;
    let output_exponent = (3 * half_exp_mask).wrapping_add(!exponent) / 2;
    (sign << (exp_bits + frac_bits)) | ((output_exponent & exp_mask) << frac_bits) | output_fraction
}

fn rvv_frec7(value: u64, exp_bits: usize, frac_bits: usize, rm: u32, flags: &mut u32) -> u64 {
    use crate::softfp::{
        FFLAG_DIVIDE_ZERO, FFLAG_INEXACT, FFLAG_INVALID_OP, FFLAG_OVERFLOW, RM_RDN, RM_RTZ, RM_RUP,
    };
    let (sign, mut exponent, mut fraction) = fp_estimate_parts(value, exp_bits, frac_bits);
    let exp_mask = (1u64 << exp_bits) - 1;
    let frac_mask = (1u64 << frac_bits) - 1;
    let quiet_bit = 1u64 << (frac_bits - 1);
    let infinity = exp_mask << frac_bits;
    let sign_field = sign << (exp_bits + frac_bits);
    if exponent == exp_mask && fraction == 0 {
        return sign_field;
    }
    if exponent == 0 && fraction == 0 {
        *flags |= FFLAG_DIVIDE_ZERO;
        return sign_field | infinity;
    }
    if exponent == exp_mask {
        if fraction & quiet_bit == 0 {
            *flags |= FFLAG_INVALID_OP;
        }
        return infinity | quiet_bit;
    }
    if exponent == 0 {
        while fraction >> (frac_bits - 1) == 0 {
            exponent = exponent.wrapping_sub(1);
            fraction <<= 1;
        }
        fraction = (fraction << 1) & frac_mask;
        if exponent != 0 && exponent != u64::MAX {
            *flags |= FFLAG_OVERFLOW | FFLAG_INEXACT;
            let greatest_finite = infinity - 1;
            let finite = rm == RM_RTZ || (rm == RM_RDN && sign == 0) || (rm == RM_RUP && sign != 0);
            return sign_field | if finite { greatest_finite } else { infinity };
        }
    }
    let index = (fraction >> (frac_bits - 7)) as usize;
    let mut output_fraction = (FREC7_TABLE[index] as u64) << (frac_bits - 7);
    let half_exp_mask = (1u64 << (exp_bits - 1)) - 1;
    let mut output_exponent = (2 * half_exp_mask).wrapping_add(!exponent);
    if output_exponent == 0 || output_exponent == u64::MAX {
        output_fraction = (output_fraction >> 1) | (1u64 << (frac_bits - 1));
        if output_exponent == u64::MAX {
            output_fraction >>= 1;
            output_exponent = 0;
        }
    }
    sign_field | ((output_exponent & exp_mask) << frac_bits) | (output_fraction & frac_mask)
}

fn fp32_to_integer_width(
    value: u32,
    width: usize,
    unsigned: bool,
    rm: u32,
    flags: &mut u32,
) -> u64 {
    use crate::softfp::{sf32, FFLAG_INEXACT, FFLAG_INVALID_OP};
    let mut local = 0;
    let converted = sf32::cvt_to_i64(value, rm, &mut local, unsigned);
    if unsigned {
        let maximum = element_mask(width);
        if local & FFLAG_INVALID_OP != 0 || converted > maximum {
            *flags |= (local & !FFLAG_INEXACT) | FFLAG_INVALID_OP;
            converted.min(maximum)
        } else {
            *flags |= local;
            converted
        }
    } else {
        let converted = converted as i64 as i128;
        let minimum = -(1i128 << (width - 1));
        let maximum = (1i128 << (width - 1)) - 1;
        if local & FFLAG_INVALID_OP != 0 || converted < minimum || converted > maximum {
            *flags |= (local & !FFLAG_INEXACT) | FFLAG_INVALID_OP;
            converted.clamp(minimum, maximum) as u64 & element_mask(width)
        } else {
            *flags |= local;
            converted as u64 & element_mask(width)
        }
    }
}

#[inline]
fn vector_width(funct3: u32) -> Option<usize> {
    match funct3 {
        0 => Some(8),
        5 => Some(16),
        6 => Some(32),
        7 => Some(64),
        _ => None,
    }
}

/// True for RVV's regular, unmasked, single-field unit-stride memory encoding
/// (nf=0, mew=0, mop=00, vm=1, lumop/sumop=00000).
#[inline]
pub(super) fn unit_stride_memory_encoding(insn: u32) -> bool {
    const SHAPE_MASK: u32 = (7 << 29) | (1 << 28) | (3 << 26) | (1 << 25) | (0x1f << 20);
    insn & SHAPE_MASK == 1 << 25
}

impl Cpu {
    #[inline]
    pub(super) fn vector_enabled(&self) -> bool {
        self.sys
            .as_ref()
            .is_none_or(|sys| sys.mstatus & MSTATUS_VS != 0)
    }

    #[inline]
    pub(super) fn vector_check(&self, insn: u32) -> Result<(), Exception> {
        if self.vector_enabled() {
            Ok(())
        } else {
            Err(Exception::IllegalInstruction { insn })
        }
    }

    #[inline]
    pub(super) fn vector_dirty(&mut self) {
        if let Some(sys) = self.sys.as_mut() {
            sys.mstatus |= MSTATUS_VS;
        }
    }

    #[inline]
    pub(super) fn exec_vector_config(&mut self, insn: u32) -> Result<(), Exception> {
        self.vector_check(insn)?;
        if funct3(insn) != 7 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let d = rd(insn);
        let s1 = rs1(insn);
        let (raw_type, avl, immediate_avl, rs1_is_zero) = if insn >> 30 == 0b11 {
            // vsetivli: AVL is the five-bit immediate in the rs1 field.
            (((insn >> 20) & 0x3ff) as u64, s1 as u64, true, false)
        } else if insn >> 31 == 0 {
            // vsetvli
            (((insn >> 20) & 0x7ff) as u64, self.x[s1], false, s1 == 0)
        } else if (insn >> 25) & 0x3f == 0 {
            // vsetvl: vtype is supplied in rs2.
            (self.x[rs2(insn)], self.x[s1], false, s1 == 0)
        } else {
            return Err(Exception::IllegalInstruction { insn });
        };

        let new_vl = self
            .vector
            .install_config(raw_type, d, rs1_is_zero, avl, immediate_avl);
        self.vector_dirty();
        self.wr(d, new_vl);
        Ok(())
    }

    #[inline(always)]
    pub(super) fn exec_vector_op(&mut self, insn: u32) -> Result<(), Exception> {
        if funct3(insn) == 7 {
            return self.exec_vector_config(insn);
        }
        self.vector_check(insn)?;
        let config = self
            .vector
            .config()
            .ok_or(Exception::IllegalInstruction { insn })?;
        let whole_register_move = funct3(insn) == 3 && insn >> 26 == 0x27;
        if !whole_register_move
            && self.vector.vstart != 0
            && self.vector.vstart >= config.vlmax as u64
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        if matches!(funct3(insn), 1 | 5) {
            self.exec_vector_float(insn, config)
        } else {
            self.exec_vector_integer(insn, config)
        }
    }

    fn scalar_float_bits(&self, reg: usize, sew: usize) -> u64 {
        if sew == 32 {
            let value = self.f[reg];
            if value >> 32 == 0xffff_ffff {
                value as u32 as u64
            } else {
                crate::softfp::sf32::QNAN as u64
            }
        } else {
            self.f[reg]
        }
    }

    fn write_scalar_float_bits(&mut self, reg: usize, sew: usize, value: u64) {
        self.f[reg] = if sew == 32 {
            0xffff_ffff_0000_0000 | value as u32 as u64
        } else {
            value
        };
    }

    fn vector_fp_binary(
        sew: usize,
        funct6: u32,
        a: u64,
        b: u64,
        rm: u32,
        flags: &mut u32,
    ) -> Option<u64> {
        if sew == 32 {
            use crate::softfp::sf32;
            let (a, b) = (a as u32, b as u32);
            Some(match funct6 {
                0x00 => sf32::add(a, b, rm, flags),
                0x02 => sf32::sub(a, b, rm, flags),
                0x04 => sf32::min(a, b, flags),
                0x06 => sf32::max(a, b, flags),
                0x08 => (a & !sf32::SIGN_MASK) | (b & sf32::SIGN_MASK),
                0x09 => (a & !sf32::SIGN_MASK) | ((!b) & sf32::SIGN_MASK),
                0x0a => (a & !sf32::SIGN_MASK) | ((a ^ b) & sf32::SIGN_MASK),
                0x20 => sf32::div(a, b, rm, flags),
                0x21 => sf32::div(b, a, rm, flags),
                0x24 => sf32::mul(a, b, rm, flags),
                0x27 => sf32::sub(b, a, rm, flags),
                _ => return None,
            } as u64)
        } else if sew == 64 {
            use crate::softfp::sf64;
            Some(match funct6 {
                0x00 => sf64::add(a, b, rm, flags),
                0x02 => sf64::sub(a, b, rm, flags),
                0x04 => sf64::min(a, b, flags),
                0x06 => sf64::max(a, b, flags),
                0x08 => (a & !sf64::SIGN_MASK) | (b & sf64::SIGN_MASK),
                0x09 => (a & !sf64::SIGN_MASK) | ((!b) & sf64::SIGN_MASK),
                0x0a => (a & !sf64::SIGN_MASK) | ((a ^ b) & sf64::SIGN_MASK),
                0x20 => sf64::div(a, b, rm, flags),
                0x21 => sf64::div(b, a, rm, flags),
                0x24 => sf64::mul(a, b, rm, flags),
                0x27 => sf64::sub(b, a, rm, flags),
                _ => return None,
            })
        } else {
            None
        }
    }

    fn vector_fp_fma(
        sew: usize,
        funct6: u32,
        old: u64,
        a: u64,
        b: u64,
        rm: u32,
        flags: &mut u32,
    ) -> Option<u64> {
        let (mut x, y, mut z) = if funct6 < 0x2c {
            // vfmadd/vfnmadd/vfmsub/vfnmsub: old*rhs +/- vs2
            (old, b, a)
        } else {
            // vfmacc/vfnmacc/vfmsac/vfnmsac: vs2*rhs +/- old
            (a, b, old)
        };
        let negate_product = matches!(funct6, 0x29 | 0x2b | 0x2d | 0x2f);
        let negate_addend = matches!(funct6, 0x29 | 0x2a | 0x2d | 0x2e);
        if sew == 32 {
            use crate::softfp::sf32;
            if negate_product {
                x ^= sf32::SIGN_MASK as u64;
            }
            if negate_addend {
                z ^= sf32::SIGN_MASK as u64;
            }
            Some(sf32::fma(x as u32, y as u32, z as u32, rm, flags) as u64)
        } else if sew == 64 {
            use crate::softfp::sf64;
            if negate_product {
                x ^= sf64::SIGN_MASK;
            }
            if negate_addend {
                z ^= sf64::SIGN_MASK;
            }
            Some(sf64::fma(x, y, z, rm, flags))
        } else {
            None
        }
    }

    fn vector_fp_compare(sew: usize, funct6: u32, a: u64, b: u64, flags: &mut u32) -> Option<bool> {
        if sew == 32 {
            use crate::softfp::sf32;
            let (a, b) = (a as u32, b as u32);
            Some(match funct6 {
                0x18 => sf32::eq_quiet(a, b, flags),
                0x19 => sf32::le(a, b, flags),
                0x1b => sf32::lt(a, b, flags),
                0x1c => !sf32::eq_quiet(a, b, flags),
                0x1d => sf32::lt(b, a, flags),
                0x1f => sf32::le(b, a, flags),
                _ => return None,
            })
        } else if sew == 64 {
            use crate::softfp::sf64;
            Some(match funct6 {
                0x18 => sf64::eq_quiet(a, b, flags),
                0x19 => sf64::le(a, b, flags),
                0x1b => sf64::lt(a, b, flags),
                0x1c => !sf64::eq_quiet(a, b, flags),
                0x1d => sf64::lt(b, a, flags),
                0x1f => sf64::le(b, a, flags),
                _ => return None,
            })
        } else {
            None
        }
    }

    fn exec_vector_float_unary(
        &mut self,
        insn: u32,
        config: VectorConfig,
        selector: usize,
    ) -> Result<(), Exception> {
        let ratio = config.lmul();
        if !validate_group(rd(insn), ratio, 1)
            || !validate_group(rs2(insn), ratio, 1)
            || ((insn >> 25) & 1 == 0 && groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ))
            || !matches!(selector, 0 | 4 | 5 | 0x10)
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let rm = if selector == 5 {
            self.get_rm(7)
                .ok_or(Exception::IllegalInstruction { insn })?
        } else if selector == 0 {
            self.get_rm(7)
                .ok_or(Exception::IllegalInstruction { insn })?
        } else {
            0
        };
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut flags = 0;
        self.vector_dirty();
        self.fp_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, config.sew);
            let value = if config.sew == 32 {
                use crate::softfp::sf32;
                match selector {
                    0 => sf32::sqrt(source as u32, rm, &mut flags) as u64,
                    4 => rvv_frsqrt7(source as u32 as u64, 8, 23, &mut flags),
                    5 => rvv_frec7(source as u32 as u64, 8, 23, rm, &mut flags),
                    0x10 => sf32::fclass(source as u32) as u64,
                    _ => unreachable!(),
                }
            } else {
                use crate::softfp::sf64;
                match selector {
                    0 => sf64::sqrt(source, rm, &mut flags),
                    4 => rvv_frsqrt7(source, 11, 52, &mut flags),
                    5 => rvv_frec7(source, 11, 52, rm, &mut flags),
                    0x10 => sf64::fclass(source) as u64,
                    _ => unreachable!(),
                }
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.fcsr |= flags & 0x1f;
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_float_convert(
        &mut self,
        insn: u32,
        config: VectorConfig,
        selector: usize,
    ) -> Result<(), Exception> {
        let (source_eew, destination_eew) = match selector {
            0x00..=0x07 => (config.sew, config.sew),
            0x08..=0x0f => (config.sew, config.sew * 2),
            0x10..=0x17 => (config.sew * 2, config.sew),
            _ => return Err(Exception::IllegalInstruction { insn }),
        };
        let supported_widths = match selector {
            0x00..=0x03 | 0x06 | 0x07 => matches!(config.sew, 32 | 64),
            0x08 | 0x09 | 0x0c | 0x0e | 0x0f => config.sew == 32,
            // Integer-to-wider-float uses f32 for SEW=16 and f64 for
            // SEW=32. This is part of GCV and does not require Zvfh.
            0x0a | 0x0b => matches!(config.sew, 16 | 32),
            // Wider-float-to-integer similarly permits f32->i16.
            0x10 | 0x11 | 0x16 | 0x17 => matches!(config.sew, 16 | 32),
            0x12..=0x15 => config.sew == 32,
            _ => false,
        };
        if !supported_widths {
            return Err(Exception::IllegalInstruction { insn });
        }
        let source_ratio = config
            .emul(source_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let destination_ratio = config
            .emul(destination_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(rd(insn), destination_ratio, 1)
            || !validate_group(rs2(insn), source_ratio, 1)
            || !destination_source_overlap_legal(
                rd(insn),
                destination_ratio,
                destination_eew,
                rs2(insn),
                source_ratio,
                source_eew,
            )
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), destination_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    source_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let operation = selector & 7;
        let rm = if matches!(operation, 6 | 7) {
            crate::softfp::RM_RTZ
        } else if selector == 0x15 {
            // Explicit round-to-odd float narrowing.
            crate::softfp::RM_RTZ
        } else {
            self.get_rm(7)
                .ok_or(Exception::IllegalInstruction { insn })?
        };
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut flags = 0;
        self.vector_dirty();
        self.fp_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, source_eew);
            let value = match selector {
                0x00 | 0x06 => {
                    if source_eew == 32 {
                        crate::softfp::sf32::cvt_to_i32(source as u32, rm, &mut flags, true) as u64
                    } else {
                        crate::softfp::sf64::cvt_to_i64(source, rm, &mut flags, true)
                    }
                }
                0x01 | 0x07 => {
                    if source_eew == 32 {
                        crate::softfp::sf32::cvt_to_i32(source as u32, rm, &mut flags, false) as u64
                    } else {
                        crate::softfp::sf64::cvt_to_i64(source, rm, &mut flags, false)
                    }
                }
                0x02 => {
                    if destination_eew == 32 {
                        crate::softfp::sf32::cvt_from_i32(source as u32, rm, &mut flags, true)
                            as u64
                    } else {
                        crate::softfp::sf64::cvt_from_i64(source, rm, &mut flags, true)
                    }
                }
                0x03 => {
                    if destination_eew == 32 {
                        crate::softfp::sf32::cvt_from_i32(source as u32, rm, &mut flags, false)
                            as u64
                    } else {
                        crate::softfp::sf64::cvt_from_i64(source, rm, &mut flags, false)
                    }
                }
                0x08 | 0x0e => crate::softfp::sf32::cvt_to_i64(source as u32, rm, &mut flags, true),
                0x09 | 0x0f => {
                    crate::softfp::sf32::cvt_to_i64(source as u32, rm, &mut flags, false)
                }
                0x0a => {
                    if destination_eew == 32 {
                        crate::softfp::sf32::cvt_from_i32(
                            source as u16 as u32,
                            rm,
                            &mut flags,
                            true,
                        ) as u64
                    } else {
                        crate::softfp::sf64::cvt_from_i32(source as u32, rm, &mut flags, true)
                    }
                }
                0x0b => {
                    if destination_eew == 32 {
                        crate::softfp::sf32::cvt_from_i32(
                            signed_element(source, source_eew) as i32 as u32,
                            rm,
                            &mut flags,
                            false,
                        ) as u64
                    } else {
                        crate::softfp::sf64::cvt_from_i32(source as u32, rm, &mut flags, false)
                    }
                }
                0x0c => crate::softfp::cvt_sf32_sf64(source as u32, &mut flags),
                0x10 | 0x16 => {
                    if source_eew == 32 {
                        fp32_to_integer_width(source as u32, destination_eew, true, rm, &mut flags)
                    } else {
                        crate::softfp::sf64::cvt_to_i32(source, rm, &mut flags, true) as u64
                    }
                }
                0x11 | 0x17 => {
                    if source_eew == 32 {
                        fp32_to_integer_width(source as u32, destination_eew, false, rm, &mut flags)
                    } else {
                        crate::softfp::sf64::cvt_to_i32(source, rm, &mut flags, false) as u64
                    }
                }
                0x12 => crate::softfp::sf32::cvt_from_i64(source, rm, &mut flags, true) as u64,
                0x13 => crate::softfp::sf32::cvt_from_i64(source, rm, &mut flags, false) as u64,
                0x14 => crate::softfp::cvt_sf64_sf32(source, rm, &mut flags) as u64,
                0x15 => crate::softfp::cvt_sf64_sf32_rod(source, &mut flags) as u64,
                _ => return Err(Exception::IllegalInstruction { insn }),
            };
            self.vector
                .write_element(rd(insn), index, destination_eew, value);
        }
        self.fcsr |= flags & 0x1f;
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_float_widen_binary(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if !matches!((config.sew, wide_eew), (32, 64)) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let format = funct3(insn);
        let vector_rhs = format == 1;
        if !matches!(format, 1 | 5) || !matches!(funct6, 0x30 | 0x32 | 0x34 | 0x36 | 0x38) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_ratio = config.lmul();
        let wide_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let wide_left = matches!(funct6, 0x34 | 0x36);
        if !validate_group(rd(insn), wide_ratio, 1)
            || !validate_group(
                rs2(insn),
                if wide_left { wide_ratio } else { narrow_ratio },
                1,
            )
            || (vector_rhs && !validate_group(rs1(insn), narrow_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                if wide_left { wide_ratio } else { narrow_ratio },
                if wide_left { wide_eew } else { config.sew },
            )
            || (vector_rhs
                && !destination_source_overlap_legal(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || (wide_left
                && vector_rhs
                && mixed_width_sources_overlap(
                    rs2(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), wide_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    if wide_left { wide_ratio } else { narrow_ratio },
                    if wide_left { wide_eew } else { config.sew },
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (vector_rhs
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        narrow_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let rm = self
            .get_rm(7)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let scalar = (!vector_rhs).then(|| self.scalar_float_bits(rs1(insn), config.sew));
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut flags = 0;
        self.vector_dirty();
        self.fp_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let left = if wide_left {
                self.vector.read_element(rs2(insn), index, wide_eew)
            } else {
                crate::softfp::cvt_sf32_sf64(
                    self.vector.read_element(rs2(insn), index, config.sew) as u32,
                    &mut flags,
                )
            };
            let narrow_right =
                scalar.unwrap_or_else(|| self.vector.read_element(rs1(insn), index, config.sew));
            let right = crate::softfp::cvt_sf32_sf64(narrow_right as u32, &mut flags);
            let value = match funct6 {
                0x30 | 0x34 => crate::softfp::sf64::add(left, right, rm, &mut flags),
                0x32 | 0x36 => crate::softfp::sf64::sub(left, right, rm, &mut flags),
                0x38 => crate::softfp::sf64::mul(left, right, rm, &mut flags),
                _ => unreachable!(),
            };
            self.vector.write_element(rd(insn), index, wide_eew, value);
        }
        self.fcsr |= flags & 0x1f;
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_float_widen_fma(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if !matches!((config.sew, wide_eew), (32, 64)) || !matches!(funct6, 0x3c..=0x3f) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let format = funct3(insn);
        let vector_rhs = format == 1;
        if !matches!(format, 1 | 5) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_ratio = config.lmul();
        let wide_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(rd(insn), wide_ratio, 1)
            || !validate_group(rs2(insn), narrow_ratio, 1)
            || (vector_rhs && !validate_group(rs1(insn), narrow_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                narrow_ratio,
                config.sew,
            )
            || (vector_rhs
                && !destination_source_overlap_legal(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || mixed_width_sources_overlap(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                narrow_ratio,
                config.sew,
            )
            || (vector_rhs
                && mixed_width_sources_overlap(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), wide_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    narrow_ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (vector_rhs
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        narrow_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let rm = self
            .get_rm(7)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let scalar = (!vector_rhs).then(|| self.scalar_float_bits(rs1(insn), config.sew));
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut flags = 0;
        self.vector_dirty();
        self.fp_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = crate::softfp::cvt_sf32_sf64(
                self.vector.read_element(rs2(insn), index, config.sew) as u32,
                &mut flags,
            );
            let narrow_b =
                scalar.unwrap_or_else(|| self.vector.read_element(rs1(insn), index, config.sew));
            let b = crate::softfp::cvt_sf32_sf64(narrow_b as u32, &mut flags);
            let old = self.vector.read_element(rd(insn), index, wide_eew);
            let mut product_left = a;
            let mut addend = old;
            if matches!(funct6, 0x3d | 0x3f) {
                product_left ^= crate::softfp::sf64::SIGN_MASK;
            }
            if matches!(funct6, 0x3d | 0x3e) {
                addend ^= crate::softfp::sf64::SIGN_MASK;
            }
            let value = crate::softfp::sf64::fma(product_left, b, addend, rm, &mut flags);
            self.vector.write_element(rd(insn), index, wide_eew, value);
        }
        self.fcsr |= flags & 0x1f;
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_float_widen_reduction(
        &mut self,
        insn: u32,
        config: VectorConfig,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if self.vector.vstart != 0
            || !matches!((config.sew, wide_eew), (32, 64))
            || !validate_group(rs2(insn), config.lmul(), 1)
            || mixed_width_sources_overlap(
                rs2(insn),
                config.lmul(),
                config.sew,
                rs1(insn),
                MASK_REGISTER_RATIO,
                wide_eew,
            )
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    config.lmul(),
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || mixed_width_sources_overlap(
                    rs1(insn),
                    MASK_REGISTER_RATIO,
                    wide_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                )))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let rm = self
            .get_rm(7)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let mut accumulator = self.vector.read_element(rs1(insn), 0, wide_eew);
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        if vl == 0 {
            self.vector.vstart = 0;
            return Ok(());
        }
        let masked = (insn >> 25) & 1 == 0;
        let mut flags = 0;
        for index in 0..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = crate::softfp::cvt_sf32_sf64(
                self.vector.read_element(rs2(insn), index, config.sew) as u32,
                &mut flags,
            );
            accumulator = crate::softfp::sf64::add(accumulator, source, rm, &mut flags);
        }
        self.vector
            .write_element(rd(insn), 0, wide_eew, accumulator);
        self.fcsr |= flags & 0x1f;
        self.vector_dirty();
        self.fp_dirty();
        self.vector.vstart = 0;
        Ok(())
    }

    #[inline(never)]
    fn exec_vector_float(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        self.fp_check(insn)?;
        let format = funct3(insn);
        let funct6 = insn >> 26;
        let selector = rs1(insn);
        let vm = (insn >> 25) & 1 != 0;

        if format == 1 && funct6 == 0x12 {
            return self.exec_vector_float_convert(insn, config, selector);
        }

        if !matches!(config.sew, 32 | 64) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let ratio = config.lmul();

        if format == 1 && funct6 == 0x13 {
            return self.exec_vector_float_unary(insn, config, selector);
        }

        if matches!(format, 1 | 5) && matches!(funct6, 0x30 | 0x32 | 0x34 | 0x36 | 0x38) {
            return self.exec_vector_float_widen_binary(insn, config, funct6);
        }
        if matches!(format, 1 | 5) && matches!(funct6, 0x3c..=0x3f) {
            return self.exec_vector_float_widen_fma(insn, config, funct6);
        }
        if format == 1 && matches!(funct6, 0x31 | 0x33) {
            return self.exec_vector_float_widen_reduction(insn, config);
        }

        // vfmv.s.f
        if format == 5 && funct6 == 0x10 && vm && rs2(insn) == 0 {
            if self.vector.vl > 0 && self.vector.vstart < self.vector.vl {
                let value = self.scalar_float_bits(selector, config.sew);
                self.vector.write_element(rd(insn), 0, config.sew, value);
            }
            self.vector.vstart = 0;
            self.vector_dirty();
            return Ok(());
        }

        // vfmv.f.s
        if format == 1 && funct6 == 0x10 && vm && selector == 0 {
            let value = self.vector.read_element(rs2(insn), 0, config.sew);
            self.write_scalar_float_bits(rd(insn), config.sew, value);
            self.vector.vstart = 0;
            self.vector_dirty();
            self.fp_dirty();
            return Ok(());
        }

        // vfmerge.vfm / vfmv.v.f
        if format == 5 && funct6 == 0x17 {
            if !validate_group(rd(insn), ratio, 1)
                || (!vm && !validate_group(rs2(insn), ratio, 1))
                || (vm && rs2(insn) != 0)
                || (!vm && groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
                || (!vm
                    && mixed_width_sources_overlap(
                        rs2(insn),
                        ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))
            {
                return Err(Exception::IllegalInstruction { insn });
            }
            let scalar = self.scalar_float_bits(selector, config.sew);
            let start = self.vector.vstart as usize;
            let vl = self.vector.vl.min(config.vlmax as u64) as usize;
            self.vector_dirty();
            for index in start..vl {
                let value = if vm || self.vector.mask_bit(index) {
                    scalar
                } else {
                    self.vector.read_element(rs2(insn), index, config.sew)
                };
                self.vector
                    .write_element(rd(insn), index, config.sew, value);
            }
            self.vector.vstart = 0;
            return Ok(());
        }

        // vfslide1up/down.vf
        if format == 5 && matches!(funct6, 0x0e | 0x0f) {
            if !validate_group(rd(insn), ratio, 1)
                || !validate_group(rs2(insn), ratio, 1)
                || (funct6 == 0x0e && rd(insn) == rs2(insn))
                || (!vm && groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
                || (!vm
                    && mixed_width_sources_overlap(
                        rs2(insn),
                        ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))
            {
                return Err(Exception::IllegalInstruction { insn });
            }
            let source = self.vector.clone();
            let scalar = self.scalar_float_bits(selector, config.sew);
            let start = self.vector.vstart as usize;
            let vl = self.vector.vl.min(config.vlmax as u64) as usize;
            self.vector_dirty();
            for index in start..vl {
                if !vm && !source.mask_bit(index) {
                    continue;
                }
                let value = if funct6 == 0x0e {
                    if index == 0 {
                        scalar
                    } else {
                        source.read_element(rs2(insn), index - 1, config.sew)
                    }
                } else if index + 1 == vl {
                    scalar
                } else {
                    source.read_element(rs2(insn), index + 1, config.sew)
                };
                self.vector
                    .write_element(rd(insn), index, config.sew, value);
            }
            self.vector.vstart = 0;
            return Ok(());
        }

        // Ordered and unordered floating-point sums. The sequential order is
        // exact for vfredosum and is one architecturally legal vfredusum tree.
        if format == 1 && matches!(funct6, 0x01 | 0x03 | 0x05 | 0x07) {
            if self.vector.vstart != 0
                || !validate_group(rs2(insn), ratio, 1)
                || (!vm
                    && (mixed_width_sources_overlap(
                        rs2(insn),
                        ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ) || mixed_width_sources_overlap(
                        selector,
                        MASK_REGISTER_RATIO,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    )))
            {
                return Err(Exception::IllegalInstruction { insn });
            }
            let rm = if matches!(funct6, 0x01 | 0x03) {
                self.get_rm(7)
                    .ok_or(Exception::IllegalInstruction { insn })?
            } else {
                0
            };
            let mut accumulator = self.vector.read_element(selector, 0, config.sew);
            let vl = self.vector.vl.min(config.vlmax as u64) as usize;
            if vl == 0 {
                self.vector.vstart = 0;
                return Ok(());
            }
            let mut flags = 0;
            for index in 0..vl {
                if !vm && !self.vector.mask_bit(index) {
                    continue;
                }
                let value = self.vector.read_element(rs2(insn), index, config.sew);
                accumulator = Self::vector_fp_binary(
                    config.sew,
                    if funct6 == 0x05 {
                        0x04
                    } else if funct6 == 0x07 {
                        0x06
                    } else {
                        0x00
                    },
                    accumulator,
                    value,
                    rm,
                    &mut flags,
                )
                .unwrap();
            }
            self.vector
                .write_element(rd(insn), 0, config.sew, accumulator);
            self.fcsr |= flags & 0x1f;
            self.vector_dirty();
            self.fp_dirty();
            return Ok(());
        }

        let comparison = matches!(funct6, 0x18 | 0x19 | 0x1b | 0x1c | 0x1d | 0x1f);
        let fused = matches!(funct6, 0x28..=0x2f);
        let ordinary = matches!(
            funct6,
            0x00 | 0x02 | 0x04 | 0x06 | 0x08 | 0x09 | 0x0a | 0x20 | 0x21 | 0x24 | 0x27
        );
        let destination_ok = if comparison {
            rd(insn) < VREG_COUNT
                && destination_source_overlap_legal(
                    rd(insn),
                    MASK_REGISTER_RATIO,
                    1,
                    rs2(insn),
                    ratio,
                    config.sew,
                )
                && (format != 1
                    || destination_source_overlap_legal(
                        rd(insn),
                        MASK_REGISTER_RATIO,
                        1,
                        selector,
                        ratio,
                        config.sew,
                    ))
        } else {
            validate_group(rd(insn), ratio, 1)
                && (vm || !groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
        };
        if (!comparison && !fused && !ordinary)
            || !matches!(format, 1 | 5)
            || !validate_group(rs2(insn), ratio, 1)
            || (format == 1 && !validate_group(selector, ratio, 1))
            || !destination_ok
            || (!vm
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (format == 1
                    && mixed_width_sources_overlap(
                        selector,
                        ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
            || (format == 1 && matches!(funct6, 0x21 | 0x27 | 0x1d | 0x1f))
        {
            return Err(Exception::IllegalInstruction { insn });
        }

        let rounding_required = fused || matches!(funct6, 0x00 | 0x02 | 0x20 | 0x21 | 0x24 | 0x27);
        let rm = if rounding_required {
            self.get_rm(7)
                .ok_or(Exception::IllegalInstruction { insn })?
        } else {
            0
        };
        let scalar = (format == 5).then(|| self.scalar_float_bits(selector, config.sew));
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut flags = 0;
        self.vector_dirty();
        self.fp_dirty();
        for index in start..vl {
            if !vm && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = scalar.unwrap_or_else(|| self.vector.read_element(selector, index, config.sew));
            if comparison {
                let result = Self::vector_fp_compare(config.sew, funct6, a, b, &mut flags)
                    .ok_or(Exception::IllegalInstruction { insn })?;
                self.write_mask_element(rd(insn), index, result);
            } else {
                let value = if fused {
                    let old = self.vector.read_element(rd(insn), index, config.sew);
                    Self::vector_fp_fma(config.sew, funct6, old, a, b, rm, &mut flags)
                } else {
                    Self::vector_fp_binary(config.sew, funct6, a, b, rm, &mut flags)
                }
                .ok_or(Exception::IllegalInstruction { insn })?;
                self.vector
                    .write_element(rd(insn), index, config.sew, value);
            }
        }
        self.fcsr |= flags & 0x1f;
        self.vector.vstart = 0;
        Ok(())
    }

    #[inline(always)]
    fn integer_groups(
        &self,
        insn: u32,
        config: VectorConfig,
        destination_is_mask: bool,
        needs_vs2: bool,
        needs_vs1: bool,
    ) -> Result<(), Exception> {
        let ratio = config.lmul();
        let d = rd(insn);
        let s2 = rs2(insn);
        let s1 = rs1(insn);
        let masked = (insn >> 25) & 1 == 0;
        let destination_ok = if destination_is_mask {
            d < VREG_COUNT
                && (!needs_vs2
                    || destination_source_overlap_legal(
                        d,
                        MASK_REGISTER_RATIO,
                        1,
                        s2,
                        ratio,
                        config.sew,
                    ))
                && (!needs_vs1
                    || destination_source_overlap_legal(
                        d,
                        MASK_REGISTER_RATIO,
                        1,
                        s1,
                        ratio,
                        config.sew,
                    ))
        } else {
            validate_group(d, ratio, 1)
                && (!masked || !groups_overlap(d, ratio, 1, 0, MASK_REGISTER_RATIO))
        };
        if !destination_ok
            || (needs_vs2 && !validate_group(s2, ratio, 1))
            || (needs_vs1 && !validate_group(s1, ratio, 1))
            || (masked
                && ((needs_vs2
                    && mixed_width_sources_overlap(
                        s2,
                        ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))
                    || (needs_vs1
                        && mixed_width_sources_overlap(
                            s1,
                            ratio,
                            config.sew,
                            0,
                            MASK_REGISTER_RATIO,
                            1,
                        ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        Ok(())
    }

    fn write_mask_element(&mut self, reg: usize, index: usize, value: bool) {
        let byte = &mut self.vector.regs[reg][index >> 3];
        let bit = 1 << (index & 7);
        if value {
            *byte |= bit;
        } else {
            *byte &= !bit;
        }
    }

    fn exec_vector_whole_move(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        let count = rs1(insn) + 1;
        let evl = count * VLEN_BITS / config.sew;
        if !matches!(count, 1 | 2 | 4 | 8)
            || (insn >> 25) & 1 == 0
            || rd(insn) % count != 0
            || rs2(insn) % count != 0
            || rd(insn) + count > VREG_COUNT
            || rs2(insn) + count > VREG_COUNT
            || self.vector.vstart as usize >= evl
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let start = self.vector.vstart as usize * (config.sew / 8);
        let bytes = count * VLEN_BYTES;
        if rd(insn) != rs2(insn) && start < bytes {
            let source = self.vector.regs;
            for offset in start..bytes {
                let source_flat = rs2(insn) * VLEN_BYTES + offset;
                let destination_flat = rd(insn) * VLEN_BYTES + offset;
                let value = source[source_flat / VLEN_BYTES][source_flat % VLEN_BYTES];
                self.vector.set_flat_byte(destination_flat, value);
            }
        }
        self.vector.vstart = 0;
        self.vector_dirty();
        Ok(())
    }

    fn exec_vector_extension(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        let selector = rs1(insn);
        let (divisor, signed) = match selector {
            2 => (8, false),
            3 => (8, true),
            4 => (4, false),
            5 => (4, true),
            6 => (2, false),
            7 => (2, true),
            _ => return Err(Exception::IllegalInstruction { insn }),
        };
        if config.sew % divisor != 0 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let source_eew = config.sew / divisor;
        if source_eew < 8 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let destination_ratio = config.lmul();
        let source_ratio = config
            .emul(source_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(rd(insn), destination_ratio, 1)
            || !validate_group(rs2(insn), source_ratio, 1)
            || !destination_source_overlap_legal(
                rd(insn),
                destination_ratio,
                config.sew,
                rs2(insn),
                source_ratio,
                source_eew,
            )
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), destination_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    source_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ))
        {
            return Err(Exception::IllegalInstruction { insn });
        }

        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, source_eew);
            let value = if signed {
                signed_element(source, source_eew) as u64
            } else {
                source
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_slide_or_gather(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
        format: u32,
    ) -> Result<(), Exception> {
        let ratio = config.lmul();
        let source_reg = rs2(insn);
        if !validate_group(rd(insn), ratio, 1)
            || !validate_group(source_reg, ratio, 1)
            || ((insn >> 25) & 1 == 0 && groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && mixed_width_sources_overlap(
                    source_reg,
                    ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let is_slide_up = funct6 == 0x0e && format != 0;
        if is_slide_up && rd(insn) == source_reg {
            return Err(Exception::IllegalInstruction { insn });
        }
        let needs_index_group = format == 0 && funct6 == 0x0c;
        if needs_index_group && !validate_group(rs1(insn), ratio, 1) {
            return Err(Exception::IllegalInstruction { insn });
        }
        if needs_index_group
            && (insn >> 25) & 1 == 0
            && mixed_width_sources_overlap(rs1(insn), ratio, config.sew, 0, MASK_REGISTER_RATIO, 1)
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        if funct6 == 0x0c
            && (groups_overlap(rd(insn), ratio, 1, source_reg, ratio)
                || (needs_index_group && groups_overlap(rd(insn), ratio, 1, rs1(insn), ratio)))
        {
            return Err(Exception::IllegalInstruction { insn });
        }

        let scalar = match format {
            3 => rs1(insn) as u64,
            4 => self.x[rs1(insn)],
            _ => 0,
        };
        // Snapshot only the architectural source group needed for overlap
        // semantics. Fractional LMUL and LMUL=1 occupy one 16-byte register;
        // cloning all 32 registers made small compiler-generated permutations
        // much more expensive than the operation itself.
        let single_group = ratio.register_span() == 1;
        let source_register = self.vector.regs[source_reg];
        let index_register = self.vector.regs[rs1(insn)];
        let source_state = (!single_group).then(|| self.vector.clone());
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();

        // Unmasked single-register slides are contiguous byte moves. Preserve
        // slideup's leading undisturbed elements and explicitly zero the
        // out-of-range active suffix for slidedown; tails remain untouched.
        if single_group && !masked && start == 0 && matches!(funct6, 0x0e | 0x0f) {
            let selected = usize::try_from(scalar).unwrap_or(usize::MAX);
            let width = config.sew / 8;
            let destination = &mut self.vector.regs[rd(insn)];
            if funct6 == 0x0e {
                let first = selected.min(vl);
                let count = vl - first;
                if count != 0 {
                    destination[first * width..vl * width]
                        .copy_from_slice(&source_register[..count * width]);
                }
            } else {
                let count = vl.min(config.vlmax.saturating_sub(selected));
                if count != 0 {
                    destination[..count * width].copy_from_slice(
                        &source_register[selected * width..(selected + count) * width],
                    );
                }
                destination[count * width..vl * width].fill(0);
            }
            self.vector.vstart = 0;
            return Ok(());
        }

        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let selected = if funct6 == 0x0c {
                // vrgather
                if format == 0 {
                    if single_group {
                        read_single_register_element(&index_register, index, config.sew)
                    } else {
                        source_state
                            .as_ref()
                            .unwrap()
                            .read_element(rs1(insn), index, config.sew)
                    }
                } else {
                    scalar
                }
            } else {
                scalar
            };
            let selected = usize::try_from(selected).unwrap_or(usize::MAX);

            let source_index = match funct6 {
                0x0c => Some(selected),
                0x0e => index.checked_sub(selected),
                0x0f => index.checked_add(selected),
                _ => unreachable!(),
            };
            if funct6 == 0x0e && index < selected {
                // slideup leaves the leading elements undisturbed.
                continue;
            }
            let value = source_index
                .filter(|&source_index| source_index < config.vlmax)
                .map_or(0, |source_index| {
                    if single_group {
                        read_single_register_element(&source_register, source_index, config.sew)
                    } else {
                        source_state.as_ref().unwrap().read_element(
                            source_reg,
                            source_index,
                            config.sew,
                        )
                    }
                });
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_gather_ei16(
        &mut self,
        insn: u32,
        config: VectorConfig,
    ) -> Result<(), Exception> {
        let data_ratio = config.lmul();
        let index_ratio = config
            .emul(16)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let d = rd(insn);
        let source = rs2(insn);
        let indices = rs1(insn);
        if !validate_group(d, data_ratio, 1)
            || !validate_group(source, data_ratio, 1)
            || !validate_group(indices, index_ratio, 1)
            || groups_overlap(d, data_ratio, 1, source, data_ratio)
            || groups_overlap(d, data_ratio, 1, indices, index_ratio)
            || (config.sew != 16 && groups_overlap(indices, index_ratio, 1, source, data_ratio))
            || ((insn >> 25) & 1 == 0 && groups_overlap(d, data_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    source,
                    data_ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || mixed_width_sources_overlap(
                    indices,
                    index_ratio,
                    16,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                )))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let selected = self.vector.read_element(indices, index, 16) as usize;
            let value = if selected < config.vlmax {
                self.vector.read_element(source, selected, config.sew)
            } else {
                0
            };
            self.vector.write_element(d, index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_narrow_shift(
        &mut self,
        insn: u32,
        config: VectorConfig,
        arithmetic: bool,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if wide_eew > 64 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let source_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let destination_ratio = config.lmul();
        let format = funct3(insn);
        if !matches!(format, 0 | 3 | 4)
            || !validate_group(rd(insn), destination_ratio, 1)
            || !validate_group(rs2(insn), source_ratio, 1)
            || (format == 0 && !validate_group(rs1(insn), destination_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                destination_ratio,
                config.sew,
                rs2(insn),
                source_ratio,
                wide_eew,
            )
            || (format == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    wide_eew,
                    rs1(insn),
                    destination_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), destination_ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    wide_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (format == 0
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        destination_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }

        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let shift = match format {
                0 => self.vector.read_element(rs1(insn), index, config.sew),
                3 => rs1(insn) as u64,
                4 => self.x[rs1(insn)],
                _ => return Err(Exception::IllegalInstruction { insn }),
            } as usize
                & (wide_eew - 1);
            let source = self.vector.read_element(rs2(insn), index, wide_eew);
            let value = if arithmetic {
                (signed_element(source, wide_eew) >> shift) as u64
            } else {
                source >> shift
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_slide1(
        &mut self,
        insn: u32,
        config: VectorConfig,
        up: bool,
    ) -> Result<(), Exception> {
        let ratio = config.lmul();
        if !validate_group(rd(insn), ratio, 1)
            || !validate_group(rs2(insn), ratio, 1)
            || (up && rd(insn) == rs2(insn))
            || ((insn >> 25) & 1 == 0 && groups_overlap(rd(insn), ratio, 1, 0, MASK_REGISTER_RATIO))
            || ((insn >> 25) & 1 == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let single_group = ratio.register_span() == 1;
        let source_register = self.vector.regs[rs2(insn)];
        let source_state = (!single_group).then(|| self.vector.clone());
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let scalar = self.x[rs1(insn)];
        self.vector_dirty();

        if single_group && !masked && start == 0 && vl != 0 {
            let width = config.sew / 8;
            let active_bytes = vl * width;
            let destination = &mut self.vector.regs[rd(insn)];
            if up {
                destination[width..active_bytes]
                    .copy_from_slice(&source_register[..active_bytes - width]);
                destination[..width].copy_from_slice(&scalar.to_le_bytes()[..width]);
            } else {
                destination[..active_bytes - width]
                    .copy_from_slice(&source_register[width..active_bytes]);
                destination[active_bytes - width..active_bytes]
                    .copy_from_slice(&scalar.to_le_bytes()[..width]);
            }
            self.vector.vstart = 0;
            return Ok(());
        }
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let value = if up {
                if index == 0 {
                    scalar
                } else if single_group {
                    read_single_register_element(&source_register, index - 1, config.sew)
                } else {
                    source_state
                        .as_ref()
                        .unwrap()
                        .read_element(rs2(insn), index - 1, config.sew)
                }
            } else if index + 1 == vl {
                scalar
            } else if single_group {
                read_single_register_element(&source_register, index + 1, config.sew)
            } else {
                source_state
                    .as_ref()
                    .unwrap()
                    .read_element(rs2(insn), index + 1, config.sew)
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_mask_logic(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        if (insn >> 25) & 1 == 0 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let d = rd(insn);
        let s2 = rs2(insn);
        let s1 = rs1(insn);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        self.vector_dirty();
        for index in start..vl {
            let a = self.vector.regs[s2][index >> 3] & (1 << (index & 7)) != 0;
            let b = self.vector.regs[s1][index >> 3] & (1 << (index & 7)) != 0;
            let value = match funct6 {
                0x18 => a & !b,
                0x19 => a & b,
                0x1a => a | b,
                0x1b => a ^ b,
                0x1c => a | !b,
                0x1d => !(a & b),
                0x1e => !(a | b),
                0x1f => !(a ^ b),
                _ => unreachable!(),
            };
            self.write_mask_element(d, index, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_mask_query(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        if self.vector.vstart != 0 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let selector = rs1(insn);
        if !matches!(selector, 0x10 | 0x11) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let vm = (insn >> 25) & 1 != 0;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut count = 0u64;
        let mut first = None;
        for index in 0..vl {
            if (vm || self.vector.mask_bit(index))
                && self.vector.regs[rs2(insn)][index >> 3] & (1 << (index & 7)) != 0
            {
                count += 1;
                first.get_or_insert(index as u64);
            }
        }
        self.wr(
            rd(insn),
            if selector == 0x10 {
                count
            } else {
                first.unwrap_or(u64::MAX)
            },
        );
        Ok(())
    }

    fn exec_vector_mask_prefix(
        &mut self,
        insn: u32,
        config: VectorConfig,
    ) -> Result<(), Exception> {
        let selector = rs1(insn);
        let d = rd(insn);
        let source = rs2(insn);
        let vm = (insn >> 25) & 1 != 0;
        if self.vector.vstart != 0
            || !matches!(selector, 1 | 2 | 3)
            || d == source
            || (!vm && d == 0)
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut found = false;
        self.vector_dirty();
        for index in 0..vl {
            if !vm && !self.vector.mask_bit(index) {
                continue;
            }
            let source_set = self.vector.regs[source][index >> 3] & (1 << (index & 7)) != 0;
            let value = if found {
                false
            } else if source_set {
                found = true;
                selector != 1 // vmsbf excludes the first set bit
            } else {
                selector != 2 // vmsof only writes the first set bit
            };
            self.write_mask_element(d, index, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_iota(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        let ratio = config.lmul();
        let d = rd(insn);
        let source = rs2(insn);
        let vm = (insn >> 25) & 1 != 0;
        if self.vector.vstart != 0
            || !validate_group(d, ratio, 1)
            || groups_overlap(d, ratio, 1, source, MASK_REGISTER_RATIO)
            || (!vm && groups_overlap(d, ratio, 1, 0, MASK_REGISTER_RATIO))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut count = 0u64;
        self.vector_dirty();
        for index in 0..vl {
            if !vm && !self.vector.mask_bit(index) {
                continue;
            }
            self.vector.write_element(d, index, config.sew, count);
            if self.vector.regs[source][index >> 3] & (1 << (index & 7)) != 0 {
                count += 1;
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_compress(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        let ratio = config.lmul();
        let d = rd(insn);
        let source = rs2(insn);
        let selector = rs1(insn);
        if self.vector.vstart != 0
            || (insn >> 25) & 1 == 0
            || !validate_group(d, ratio, 1)
            || !validate_group(source, ratio, 1)
            || groups_overlap(d, ratio, 1, source, ratio)
            || groups_overlap(d, ratio, 1, selector, MASK_REGISTER_RATIO)
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut output = 0;
        self.vector_dirty();
        for index in 0..vl {
            if self.vector.regs[selector][index >> 3] & (1 << (index & 7)) != 0 {
                let value = self.vector.read_element(source, index, config.sew);
                self.vector.write_element(d, output, config.sew, value);
                output += 1;
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_reduction(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        if self.vector.vstart != 0
            || !validate_group(rs2(insn), config.lmul(), 1)
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    config.lmul(),
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || mixed_width_sources_overlap(
                    rs1(insn),
                    MASK_REGISTER_RATIO,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                )))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let mut accumulator = self.vector.read_element(rs1(insn), 0, config.sew);
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        if vl == 0 {
            self.vector.vstart = 0;
            return Ok(());
        }
        let masked = (insn >> 25) & 1 == 0;
        for index in 0..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let value = self.vector.read_element(rs2(insn), index, config.sew);
            accumulator = match funct6 {
                0x00 => accumulator.wrapping_add(value),
                0x01 => accumulator & value,
                0x02 => accumulator | value,
                0x03 => accumulator ^ value,
                0x04 => accumulator.min(value),
                0x05 => {
                    if signed_element(accumulator, config.sew) <= signed_element(value, config.sew)
                    {
                        accumulator
                    } else {
                        value
                    }
                }
                0x06 => accumulator.max(value),
                0x07 => {
                    if signed_element(accumulator, config.sew) >= signed_element(value, config.sew)
                    {
                        accumulator
                    } else {
                        value
                    }
                }
                _ => unreachable!(),
            };
        }
        self.vector
            .write_element(rd(insn), 0, config.sew, accumulator);
        self.vector_dirty();
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_widen_reduction(
        &mut self,
        insn: u32,
        config: VectorConfig,
        signed: bool,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if self.vector.vstart != 0
            || wide_eew > 64
            || !validate_group(rs2(insn), config.lmul(), 1)
            || mixed_width_sources_overlap(
                rs2(insn),
                config.lmul(),
                config.sew,
                rs1(insn),
                MASK_REGISTER_RATIO,
                wide_eew,
            )
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    config.lmul(),
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || mixed_width_sources_overlap(
                    rs1(insn),
                    MASK_REGISTER_RATIO,
                    wide_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                )))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let wide_mask = element_mask(wide_eew);
        let mut accumulator = self.vector.read_element(rs1(insn), 0, wide_eew);
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        if vl == 0 {
            self.vector.vstart = 0;
            return Ok(());
        }
        let masked = (insn >> 25) & 1 == 0;
        for index in 0..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, config.sew);
            accumulator = if signed {
                (signed_narrow(accumulator, wide_eew) + signed_narrow(source, config.sew)) as u64
            } else {
                accumulator.wrapping_add(source)
            } & wide_mask;
        }
        self.vector
            .write_element(rd(insn), 0, wide_eew, accumulator);
        self.vector_dirty();
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_multiply_divide(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
        vector_rhs: bool,
    ) -> Result<(), Exception> {
        self.integer_groups(insn, config, false, true, vector_rhs)?;
        let mask = element_mask(config.sew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & mask;
            let signed_a = signed_element(a, config.sew);
            let signed_b = signed_element(b, config.sew);
            let value = match funct6 {
                0x20 => a.checked_div(b).unwrap_or(mask),
                0x21 => {
                    if signed_b == 0 {
                        -1i64 as u64
                    } else {
                        signed_a.wrapping_div(signed_b) as u64
                    }
                }
                0x22 => {
                    if b == 0 {
                        a
                    } else {
                        a % b
                    }
                }
                0x23 => {
                    if signed_b == 0 {
                        a
                    } else {
                        signed_a.wrapping_rem(signed_b) as u64
                    }
                }
                0x24 => (((a as u128) * (b as u128)) >> config.sew) as u64,
                0x25 => a.wrapping_mul(b),
                0x26 => (((signed_a as i128) * (b as i128)) >> config.sew) as u64,
                0x27 => (((signed_a as i128) * (signed_b as i128)) >> config.sew) as u64,
                _ => unreachable!(),
            } & mask;
            self.vector
                .write_element(rd(insn), index, config.sew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_carry(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let format = funct3(insn);
        let vector_rhs = format == 0;
        if !matches!(format, 0 | 3 | 4) || (matches!(funct6, 0x12 | 0x13) && format == 3) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let ratio = config.lmul();
        let d = rd(insn);
        let s2 = rs2(insn);
        let s1 = rs1(insn);
        let vm = (insn >> 25) & 1 != 0;
        let mask_result = matches!(funct6, 0x11 | 0x13);
        if !validate_group(s2, ratio, 1)
            || (vector_rhs && !validate_group(s1, ratio, 1))
            || (!vm
                && (mixed_width_sources_overlap(s2, ratio, config.sew, 0, MASK_REGISTER_RATIO, 1)
                    || (vector_rhs
                        && mixed_width_sources_overlap(
                            s1,
                            ratio,
                            config.sew,
                            0,
                            MASK_REGISTER_RATIO,
                            1,
                        ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        if mask_result {
            if !destination_source_overlap_legal(d, MASK_REGISTER_RATIO, 1, s2, ratio, config.sew)
                || (vector_rhs
                    && !destination_source_overlap_legal(
                        d,
                        MASK_REGISTER_RATIO,
                        1,
                        s1,
                        ratio,
                        config.sew,
                    ))
            {
                return Err(Exception::IllegalInstruction { insn });
            }
        } else if vm
            || !validate_group(d, ratio, 1)
            || groups_overlap(d, ratio, 1, 0, GroupRatio { num: 1, den: 1 })
        {
            // vadc/vsbc always consume v0 as carry/borrow input (vm=0).
            return Err(Exception::IllegalInstruction { insn });
        }

        let width_mask = element_mask(config.sew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        self.vector_dirty();
        for index in start..vl {
            let a = self.vector.read_element(s2, index, config.sew);
            let b = match format {
                0 => self.vector.read_element(s1, index, config.sew),
                3 => simm5(s1),
                4 => self.x[s1],
                _ => unreachable!(),
            } & width_mask;
            let carry = (!vm && self.vector.mask_bit(index)) as u64;
            let subtract = matches!(funct6, 0x12 | 0x13);
            if mask_result {
                let value = if subtract {
                    a < b || (carry != 0 && a == b)
                } else {
                    (a as u128) + (b as u128) + (carry as u128) > width_mask as u128
                };
                self.write_mask_element(d, index, value);
            } else {
                let value = if subtract {
                    a.wrapping_sub(b).wrapping_sub(carry)
                } else {
                    a.wrapping_add(b).wrapping_add(carry)
                };
                self.vector
                    .write_element(d, index, config.sew, value & width_mask);
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_widen_add_sub(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if wide_eew > 64 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_ratio = config.lmul();
        let wide_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let format = funct3(insn);
        let vector_rhs = format == 2;
        if !matches!(format, 2 | 6) || !matches!(funct6, 0x30..=0x37) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let wide_left = funct6 >= 0x34;
        let signed = funct6 & 1 != 0;
        let subtract = funct6 & 2 != 0;
        if !validate_group(rd(insn), wide_ratio, 1)
            || !validate_group(
                rs2(insn),
                if wide_left { wide_ratio } else { narrow_ratio },
                1,
            )
            || (vector_rhs && !validate_group(rs1(insn), narrow_ratio, 1))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), wide_ratio, 1, 0, MASK_REGISTER_RATIO))
            || !destination_source_overlap_legal(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                if wide_left { wide_ratio } else { narrow_ratio },
                if wide_left { wide_eew } else { config.sew },
            )
            || (vector_rhs
                && !destination_source_overlap_legal(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || (wide_left
                && vector_rhs
                && mixed_width_sources_overlap(
                    rs2(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    if wide_left { wide_ratio } else { narrow_ratio },
                    if wide_left { wide_eew } else { config.sew },
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (vector_rhs
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        narrow_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_mask = element_mask(config.sew);
        let wide_mask = element_mask(wide_eew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(
                rs2(insn),
                index,
                if wide_left { wide_eew } else { config.sew },
            );
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & narrow_mask;
            let value = if signed {
                let a = if wide_left {
                    signed_narrow(a, wide_eew)
                } else {
                    signed_narrow(a, config.sew)
                };
                let b = signed_narrow(b, config.sew);
                (if subtract { a - b } else { a + b }) as u64
            } else if subtract {
                a.wrapping_sub(b)
            } else {
                a.wrapping_add(b)
            } & wide_mask;
            self.vector.write_element(rd(insn), index, wide_eew, value);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_widen_multiply(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if wide_eew > 64 || !matches!(funct6, 0x38 | 0x3a | 0x3b) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let format = funct3(insn);
        let vector_rhs = format == 2;
        if !matches!(format, 2 | 6) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_ratio = config.lmul();
        let wide_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(rd(insn), wide_ratio, 1)
            || !validate_group(rs2(insn), narrow_ratio, 1)
            || (vector_rhs && !validate_group(rs1(insn), narrow_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                narrow_ratio,
                config.sew,
            )
            || (vector_rhs
                && !destination_source_overlap_legal(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), wide_ratio, 1, 0, GroupRatio { num: 1, den: 1 }))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    narrow_ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (vector_rhs
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        narrow_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_mask = element_mask(config.sew);
        let wide_mask = element_mask(wide_eew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & narrow_mask;
            let product = match funct6 {
                0x38 => (a as u128) * (b as u128),
                0x3a => (signed_narrow(a, config.sew) * (b as i128)) as u128,
                0x3b => (signed_narrow(a, config.sew) * signed_narrow(b, config.sew)) as u128,
                _ => unreachable!(),
            };
            self.vector
                .write_element(rd(insn), index, wide_eew, product as u64 & wide_mask);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_integer_madd(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let format = funct3(insn);
        let vector_rhs = format == 2;
        if !matches!(format, 2 | 6) || !matches!(funct6, 0x29 | 0x2b | 0x2d | 0x2f) {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.integer_groups(insn, config, false, true, vector_rhs)?;
        let width_mask = element_mask(config.sew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, config.sew);
            let operand = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & width_mask;
            let old = self.vector.read_element(rd(insn), index, config.sew);
            let product = old.wrapping_mul(operand);
            let value = match funct6 {
                0x29 => product.wrapping_add(source),
                0x2b => source.wrapping_sub(product),
                0x2d => source.wrapping_mul(operand).wrapping_add(old),
                0x2f => old.wrapping_sub(source.wrapping_mul(operand)),
                _ => unreachable!(),
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value & width_mask);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_widen_madd(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        let format = funct3(insn);
        let vector_rhs = format == 2;
        if wide_eew > 64
            || !matches!(format, 2 | 6)
            || !matches!(funct6, 0x3c..=0x3f)
            || (funct6 == 0x3e && vector_rhs)
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_ratio = config.lmul();
        let wide_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(rd(insn), wide_ratio, 1)
            || !validate_group(rs2(insn), narrow_ratio, 1)
            || (vector_rhs && !validate_group(rs1(insn), narrow_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                narrow_ratio,
                config.sew,
            )
            || (vector_rhs
                && !destination_source_overlap_legal(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || mixed_width_sources_overlap(
                rd(insn),
                wide_ratio,
                wide_eew,
                rs2(insn),
                narrow_ratio,
                config.sew,
            )
            || (vector_rhs
                && mixed_width_sources_overlap(
                    rd(insn),
                    wide_ratio,
                    wide_eew,
                    rs1(insn),
                    narrow_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(rd(insn), wide_ratio, 1, 0, GroupRatio { num: 1, den: 1 }))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    narrow_ratio,
                    config.sew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (vector_rhs
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        narrow_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let narrow_mask = element_mask(config.sew);
        let wide_mask = element_mask(wide_eew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & narrow_mask;
            let product = match funct6 {
                0x3c => (a as u128) * (b as u128),
                0x3d => (signed_narrow(a, config.sew) * signed_narrow(b, config.sew)) as u128,
                // The signedness suffixes follow assembler operand order:
                // vs1/rs1 first, then vs2.  Our local operands are b then a.
                0x3e => (signed_narrow(a, config.sew) * b as i128) as u128,
                0x3f => (a as i128 * signed_narrow(b, config.sew)) as u128,
                _ => unreachable!(),
            };
            let old = self.vector.read_element(rd(insn), index, wide_eew);
            self.vector.write_element(
                rd(insn),
                index,
                wide_eew,
                old.wrapping_add(product as u64) & wide_mask,
            );
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_average(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
    ) -> Result<(), Exception> {
        let format = funct3(insn);
        let vector_rhs = format == 2;
        if !matches!(format, 2 | 6) || !matches!(funct6, 0x08..=0x0b) {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.integer_groups(insn, config, false, true, vector_rhs)?;
        let width_mask = element_mask(config.sew);
        let signed = funct6 & 1 != 0;
        let subtract = funct6 & 2 != 0;
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & width_mask;
            let value = if signed {
                let intermediate = if subtract {
                    signed_narrow(a, config.sew) - signed_narrow(b, config.sew)
                } else {
                    signed_narrow(a, config.sew) + signed_narrow(b, config.sew)
                };
                fixed_round_signed(intermediate, 1, self.vector.vxrm) as u64
            } else if subtract {
                // The unsigned subtract is defined in SEW+1 two's-complement
                // precision so a negative difference rounds arithmetically.
                fixed_round_signed(a as i128 - b as i128, 1, self.vector.vxrm) as u64
            } else {
                fixed_round_unsigned(a as u128 + b as u128, 1, self.vector.vxrm) as u64
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value & width_mask);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_scaling_shift(
        &mut self,
        insn: u32,
        config: VectorConfig,
        arithmetic: bool,
    ) -> Result<(), Exception> {
        let format = funct3(insn);
        let vector_rhs = format == 0;
        if !matches!(format, 0 | 3 | 4) {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.integer_groups(insn, config, false, true, vector_rhs)?;
        let width_mask = element_mask(config.sew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let source = self.vector.read_element(rs2(insn), index, config.sew);
            let shift = match format {
                0 => self.vector.read_element(rs1(insn), index, config.sew),
                3 => rs1(insn) as u64,
                4 => self.x[rs1(insn)],
                _ => unreachable!(),
            } as usize
                & (config.sew - 1);
            let value = if arithmetic {
                fixed_round_signed(signed_narrow(source, config.sew), shift, self.vector.vxrm)
                    as u64
            } else {
                fixed_round_unsigned(source as u128, shift, self.vector.vxrm) as u64
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value & width_mask);
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_fractional_multiply(
        &mut self,
        insn: u32,
        config: VectorConfig,
    ) -> Result<(), Exception> {
        let format = funct3(insn);
        let vector_rhs = format == 0;
        if !matches!(format, 0 | 4) {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.integer_groups(insn, config, false, true, vector_rhs)?;
        let width_mask = element_mask(config.sew);
        let minimum = -(1i128 << (config.sew - 1));
        let maximum = (1i128 << (config.sew - 1)) - 1;
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut saturated = false;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let a = self.vector.read_element(rs2(insn), index, config.sew);
            let b = if vector_rhs {
                self.vector.read_element(rs1(insn), index, config.sew)
            } else {
                self.x[rs1(insn)]
            } & width_mask;
            let product = signed_narrow(a, config.sew) * signed_narrow(b, config.sew);
            let rounded = fixed_round_signed(product, config.sew - 1, self.vector.vxrm);
            let value = rounded.clamp(minimum, maximum);
            saturated |= value != rounded;
            self.vector
                .write_element(rd(insn), index, config.sew, value as u64 & width_mask);
        }
        self.vector.vxsat |= saturated;
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_vector_clip(
        &mut self,
        insn: u32,
        config: VectorConfig,
        signed: bool,
    ) -> Result<(), Exception> {
        let wide_eew = config.sew * 2;
        if wide_eew > 64 {
            return Err(Exception::IllegalInstruction { insn });
        }
        let source_ratio = config
            .emul(wide_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        let destination_ratio = config.lmul();
        let format = funct3(insn);
        if !matches!(format, 0 | 3 | 4)
            || !validate_group(rd(insn), destination_ratio, 1)
            || !validate_group(rs2(insn), source_ratio, 1)
            || (format == 0 && !validate_group(rs1(insn), destination_ratio, 1))
            || !destination_source_overlap_legal(
                rd(insn),
                destination_ratio,
                config.sew,
                rs2(insn),
                source_ratio,
                wide_eew,
            )
            || (format == 0
                && mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    wide_eew,
                    rs1(insn),
                    destination_ratio,
                    config.sew,
                ))
            || ((insn >> 25) & 1 == 0
                && groups_overlap(
                    rd(insn),
                    destination_ratio,
                    1,
                    0,
                    GroupRatio { num: 1, den: 1 },
                ))
            || ((insn >> 25) & 1 == 0
                && (mixed_width_sources_overlap(
                    rs2(insn),
                    source_ratio,
                    wide_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) || (format == 0
                    && mixed_width_sources_overlap(
                        rs1(insn),
                        destination_ratio,
                        config.sew,
                        0,
                        MASK_REGISTER_RATIO,
                        1,
                    ))))
        {
            return Err(Exception::IllegalInstruction { insn });
        }
        let destination_mask = element_mask(config.sew);
        let unsigned_maximum = destination_mask as u128;
        let signed_minimum = -(1i128 << (config.sew - 1));
        let signed_maximum = (1i128 << (config.sew - 1)) - 1;
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let masked = (insn >> 25) & 1 == 0;
        let mut saturated = false;
        self.vector_dirty();
        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let shift = match format {
                0 => self.vector.read_element(rs1(insn), index, config.sew),
                3 => rs1(insn) as u64,
                4 => self.x[rs1(insn)],
                _ => unreachable!(),
            } as usize
                & (wide_eew - 1);
            let source = self.vector.read_element(rs2(insn), index, wide_eew);
            let value = if signed {
                let rounded =
                    fixed_round_signed(signed_narrow(source, wide_eew), shift, self.vector.vxrm);
                let clipped = rounded.clamp(signed_minimum, signed_maximum);
                saturated |= clipped != rounded;
                clipped as u64
            } else {
                let rounded = fixed_round_unsigned(source as u128, shift, self.vector.vxrm);
                let clipped = rounded.min(unsigned_maximum);
                saturated |= clipped != rounded;
                clipped as u64
            };
            self.vector
                .write_element(rd(insn), index, config.sew, value & destination_mask);
        }
        self.vector.vxsat |= saturated;
        self.vector.vstart = 0;
        Ok(())
    }

    #[inline(never)]
    fn exec_vector_integer(&mut self, insn: u32, config: VectorConfig) -> Result<(), Exception> {
        let format = funct3(insn);
        let funct6 = insn >> 26;
        let vm = (insn >> 25) & 1 != 0;
        let d = rd(insn);
        let s2 = rs2(insn);
        let s1 = rs1(insn);

        // Whole-register moves use vstart*SEW as their byte restart point.
        if format == 3 && funct6 == 0x27 {
            return self.exec_vector_whole_move(insn, config);
        }

        // Scalar/vector element moves.
        if format == 6 && funct6 == 0x10 && vm && s2 == 0 {
            if self.vector.vl > 0 && self.vector.vstart < self.vector.vl {
                self.vector.write_element(d, 0, config.sew, self.x[s1]);
            }
            self.vector.vstart = 0;
            self.vector_dirty();
            return Ok(());
        }
        if format == 2 && funct6 == 0x10 && vm && s1 == 0 {
            let value = self.vector.read_element(s2, 0, config.sew);
            self.wr(d, signed_element(value, config.sew) as u64);
            self.vector.vstart = 0;
            return Ok(());
        }

        // The single-width ALU/compare/move encodings are the bulk of
        // compiler-generated vector code. Dispatch them before the uncommon
        // widening, mask, fixed-point, and reduction families so the common
        // path does not traverse the complete base-V decoder.
        let common_binary = match funct6 {
            0x00 | 0x09 | 0x0a | 0x0b | 0x18 | 0x19 | 0x1c | 0x1d | 0x20 | 0x21 | 0x25 | 0x28
            | 0x29 => matches!(format, 0 | 3 | 4),
            0x02 | 0x04 | 0x05 | 0x06 | 0x07 | 0x1a | 0x1b | 0x22 | 0x23 => {
                matches!(format, 0 | 4)
            }
            0x03 | 0x1e | 0x1f => matches!(format, 3 | 4),
            0x17 => matches!(format, 0 | 3 | 4),
            _ => false,
        };
        if common_binary {
            return self.exec_vector_integer_binary(insn, config, funct6, format);
        }

        // Cross-lane permutations are the other frequent compiler primitive.
        if format == 6 && matches!(funct6, 0x0e | 0x0f) {
            return self.exec_vector_slide1(insn, config, funct6 == 0x0e);
        }
        if format == 0 && funct6 == 0x0e {
            return self.exec_vector_gather_ei16(insn, config);
        }
        if matches!(funct6, 0x0c | 0x0e | 0x0f) && matches!(format, 0 | 3 | 4) {
            return self.exec_vector_slide_or_gather(insn, config, funct6, format);
        }

        // Integer extension family.
        if format == 2 && funct6 == 0x12 {
            return self.exec_vector_extension(insn, config);
        }

        // vid.v
        if format == 2 && funct6 == 0x14 && s2 == 0 && s1 == 0x11 {
            self.integer_groups(insn, config, false, false, false)?;
            let start = self.vector.vstart as usize;
            let vl = self.vector.vl.min(config.vlmax as u64) as usize;
            self.vector_dirty();
            for index in start..vl {
                if !vm && !self.vector.mask_bit(index) {
                    continue;
                }
                self.vector
                    .write_element(d, index, config.sew, index as u64);
            }
            self.vector.vstart = 0;
            return Ok(());
        }

        // Scalar mask queries, mask-prefix generation, and mask iota.
        if format == 2 && funct6 == 0x10 && matches!(s1, 0x10 | 0x11) {
            return self.exec_vector_mask_query(insn, config);
        }
        if format == 2 && funct6 == 0x14 && matches!(s1, 1 | 2 | 3) {
            return self.exec_vector_mask_prefix(insn, config);
        }
        if format == 2 && funct6 == 0x14 && s1 == 0x10 {
            return self.exec_vector_iota(insn, config);
        }
        if format == 2 && funct6 == 0x17 {
            return self.exec_vector_compress(insn, config);
        }

        // Mask-register boolean operations.
        if format == 2 && matches!(funct6, 0x18..=0x1f) {
            return self.exec_vector_mask_logic(insn, config, funct6);
        }

        // Integer reductions.
        if format == 2 && matches!(funct6, 0x00..=0x07) {
            return self.exec_vector_reduction(insn, config, funct6);
        }
        if format == 0 && matches!(funct6, 0x30 | 0x31) {
            return self.exec_vector_widen_reduction(insn, config, funct6 == 0x31);
        }

        // Add-with-carry/subtract-with-borrow and their mask-producing forms.
        if matches!(funct6, 0x10..=0x13) && matches!(format, 0 | 3 | 4) {
            return self.exec_vector_carry(insn, config, funct6);
        }

        // Integer multiply/divide family.
        if matches!(format, 2 | 6) && matches!(funct6, 0x20..=0x27) {
            return self.exec_vector_multiply_divide(insn, config, funct6, format == 2);
        }

        // Widening integer arithmetic.
        if matches!(format, 2 | 6) && matches!(funct6, 0x30..=0x37) {
            return self.exec_vector_widen_add_sub(insn, config, funct6);
        }
        if matches!(format, 2 | 6) && matches!(funct6, 0x38 | 0x3a | 0x3b) {
            return self.exec_vector_widen_multiply(insn, config, funct6);
        }
        if matches!(format, 2 | 6) && matches!(funct6, 0x3c..=0x3f) {
            return self.exec_vector_widen_madd(insn, config, funct6);
        }

        // Single-width multiply-add and fixed-point families.
        if matches!(format, 2 | 6) && matches!(funct6, 0x29 | 0x2b | 0x2d | 0x2f) {
            return self.exec_vector_integer_madd(insn, config, funct6);
        }
        if matches!(format, 2 | 6) && matches!(funct6, 0x08..=0x0b) {
            return self.exec_vector_average(insn, config, funct6);
        }
        if matches!(format, 0 | 4) && funct6 == 0x27 {
            return self.exec_vector_fractional_multiply(insn, config);
        }
        if matches!(format, 0 | 3 | 4) && matches!(funct6, 0x2a | 0x2b) {
            return self.exec_vector_scaling_shift(insn, config, funct6 == 0x2b);
        }

        // vslide1up/down.vx
        if format == 6 && matches!(funct6, 0x0e | 0x0f) {
            return self.exec_vector_slide1(insn, config, funct6 == 0x0e);
        }

        if format == 0 && funct6 == 0x0e {
            return self.exec_vector_gather_ei16(insn, config);
        }

        // Permutations used by compiler-generated loop setup.
        if matches!(funct6, 0x0c | 0x0e | 0x0f) && matches!(format, 0 | 3 | 4) {
            return self.exec_vector_slide_or_gather(insn, config, funct6, format);
        }

        // Narrowing logical/arithmetic shifts take a 2*SEW source.
        if matches!(funct6, 0x2c | 0x2d) && matches!(format, 0 | 3 | 4) {
            return self.exec_vector_narrow_shift(insn, config, funct6 == 0x2d);
        }
        if matches!(funct6, 0x2e | 0x2f) && matches!(format, 0 | 3 | 4) {
            return self.exec_vector_clip(insn, config, funct6 == 0x2f);
        }

        Err(Exception::IllegalInstruction { insn })
    }

    #[inline(never)]
    fn exec_vector_integer_binary(
        &mut self,
        insn: u32,
        config: VectorConfig,
        funct6: u32,
        format: u32,
    ) -> Result<(), Exception> {
        let vm = (insn >> 25) & 1 != 0;
        let d = rd(insn);
        let s2 = rs2(insn);
        let s1 = rs1(insn);

        let comparison = matches!(funct6, 0x18..=0x1f);
        let move_or_merge = funct6 == 0x17 && matches!(format, 0 | 3 | 4);
        let supported_binary = match funct6 {
            0x00 | 0x09 | 0x0a | 0x0b | 0x18 | 0x19 | 0x1c | 0x1d | 0x20 | 0x21 | 0x25 | 0x28
            | 0x29 => matches!(format, 0 | 3 | 4),
            0x02 | 0x04 | 0x05 | 0x06 | 0x07 | 0x1a | 0x1b | 0x22 | 0x23 => {
                matches!(format, 0 | 4)
            }
            0x03 | 0x1e | 0x1f => matches!(format, 3 | 4),
            _ => false,
        };
        if !supported_binary && !move_or_merge {
            return Err(Exception::IllegalInstruction { insn });
        }

        // OPIVV has two vector operands. OPIVX/OPIVI have a scalar/immediate.
        // vmv.v.* encodes its source specially (vs2 must be zero).
        // OPIVV always reads vs1. For vmv.v.v it is the sole vector source;
        // for vmerge.vvm both vs1 and vs2 are data sources.
        let needs_vs1 = format == 0;
        let needs_vs2 = !move_or_merge || !vm;
        if move_or_merge && vm && s2 != 0 {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.integer_groups(insn, config, comparison, needs_vs2, needs_vs1)?;

        let mask = element_mask(config.sew);
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        let mut saturated = false;
        self.vector_dirty();

        // Bitwise vector-vector operations have no cross-lane carries and can
        // operate on the active register bytes directly. Same-ratio legal
        // groups are either identical or disjoint, so in-place destinations
        // retain the element-loop overlap semantics.
        if vm && start == 0 && format == 0 && matches!(funct6, 0x09..=0x0b) {
            let byte_count = vl * (config.sew / 8);
            let destination_offset = d * VLEN_BYTES;
            let source2_offset = s2 * VLEN_BYTES;
            let source1_offset = s1 * VLEN_BYTES;
            let base = self.vector.regs.as_mut_ptr().cast::<u8>();
            let mut offset = 0;
            unsafe {
                // SAFETY: group validation above bounds all three flattened
                // ranges. Each chunk reads both operands before writing the
                // corresponding destination chunk.
                while offset + 8 <= byte_count {
                    let a =
                        core::ptr::read_unaligned(base.add(source2_offset + offset).cast::<u64>());
                    let b =
                        core::ptr::read_unaligned(base.add(source1_offset + offset).cast::<u64>());
                    let value = match funct6 {
                        0x09 => a & b,
                        0x0a => a | b,
                        _ => a ^ b,
                    };
                    core::ptr::write_unaligned(
                        base.add(destination_offset + offset).cast::<u64>(),
                        value,
                    );
                    offset += 8;
                }
                while offset < byte_count {
                    let a = core::ptr::read(base.add(source2_offset + offset));
                    let b = core::ptr::read(base.add(source1_offset + offset));
                    core::ptr::write(
                        base.add(destination_offset + offset),
                        match funct6 {
                            0x09 => a & b,
                            0x0a => a | b,
                            _ => a ^ b,
                        },
                    );
                    offset += 1;
                }
            }
            self.vector.vstart = 0;
            return Ok(());
        }

        for index in start..vl {
            // vmerge consumes v0 as data selection rather than as an
            // instruction predicate. vmv (vm=1) writes every active element.
            if !move_or_merge && !vm && !self.vector.mask_bit(index) {
                continue;
            }
            let a = if move_or_merge && vm {
                0
            } else {
                self.vector.read_element(s2, index, config.sew)
            };
            let b = match format {
                0 => self.vector.read_element(s1, index, config.sew),
                3 => simm5(s1),
                4 => self.x[s1],
                _ => unreachable!(),
            } & mask;

            if comparison {
                let result = match funct6 {
                    0x18 => a == b,
                    0x19 => a != b,
                    0x1a => a < b,
                    0x1b => signed_element(a, config.sew) < signed_element(b, config.sew),
                    0x1c => a <= b,
                    0x1d => signed_element(a, config.sew) <= signed_element(b, config.sew),
                    0x1e => a > b,
                    0x1f => signed_element(a, config.sew) > signed_element(b, config.sew),
                    _ => unreachable!(),
                };
                self.write_mask_element(d, index, result);
                continue;
            }

            let value = if move_or_merge {
                if vm || self.vector.mask_bit(index) {
                    b
                } else {
                    a
                }
            } else {
                match funct6 {
                    0x00 => a.wrapping_add(b),
                    0x02 => a.wrapping_sub(b),
                    0x03 => b.wrapping_sub(a),
                    0x04 => a.min(b),
                    0x05 => {
                        if signed_element(a, config.sew) < signed_element(b, config.sew) {
                            a
                        } else {
                            b
                        }
                    }
                    0x06 => a.max(b),
                    0x07 => {
                        if signed_element(a, config.sew) > signed_element(b, config.sew) {
                            a
                        } else {
                            b
                        }
                    }
                    0x09 => a & b,
                    0x0a => a | b,
                    0x0b => a ^ b,
                    0x20 => {
                        let value = (a as u128) + (b as u128);
                        if value > mask as u128 {
                            saturated = true;
                            mask
                        } else {
                            value as u64
                        }
                    }
                    0x21 => {
                        let value = signed_element(a, config.sew) as i128
                            + signed_element(b, config.sew) as i128;
                        let minimum = -(1i128 << (config.sew - 1));
                        let maximum = (1i128 << (config.sew - 1)) - 1;
                        if value < minimum || value > maximum {
                            saturated = true;
                        }
                        value.clamp(minimum, maximum) as u64
                    }
                    0x22 => {
                        if a < b {
                            saturated = true;
                            0
                        } else {
                            a - b
                        }
                    }
                    0x23 => {
                        let value = signed_element(a, config.sew) as i128
                            - signed_element(b, config.sew) as i128;
                        let minimum = -(1i128 << (config.sew - 1));
                        let maximum = (1i128 << (config.sew - 1)) - 1;
                        if value < minimum || value > maximum {
                            saturated = true;
                        }
                        value.clamp(minimum, maximum) as u64
                    }
                    0x25 => a << (b as usize & (config.sew - 1)),
                    0x28 => a >> (b as usize & (config.sew - 1)),
                    0x29 => {
                        (signed_element(a, config.sew) >> (b as usize & (config.sew - 1))) as u64
                    }
                    _ => unreachable!(),
                }
            } & mask;
            self.vector.write_element(d, index, config.sew, value);
        }
        if saturated {
            self.vector.vxsat = true;
        }
        self.vector.vstart = 0;
        Ok(())
    }

    #[inline(always)]
    fn vector_load_element<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u64,
        eew: usize,
    ) -> Result<u64, Exception> {
        match eew {
            8 => self.ld::<B, 1>(bus, address),
            16 => self.ld::<B, 2>(bus, address),
            32 => self.ld::<B, 4>(bus, address),
            64 => self.ld::<B, 8>(bus, address),
            _ => unreachable!("validated vector EEW"),
        }
    }

    #[inline(always)]
    fn vector_store_element<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u64,
        eew: usize,
        value: u64,
    ) -> Result<(), Exception> {
        match eew {
            8 => self.st::<B, 1>(bus, address, value),
            16 => self.st::<B, 2>(bus, address, value),
            32 => self.st::<B, 4>(bus, address, value),
            64 => self.st::<B, 8>(bus, address, value),
            _ => unreachable!("validated vector EEW"),
        }
    }

    /// Return a direct-RAM pointer for one architecturally contiguous vector
    /// transfer. A fused TLB capability proves permissions and direct RAM for
    /// the complete 4 KiB page; callers divide larger operations into page
    /// spans so each translation retains an exact architectural fault point.
    #[inline(always)]
    fn vector_direct_span<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u64,
        byte_count: usize,
        store: bool,
    ) -> Result<Option<usize>, Exception> {
        if !self.interpreter_fused_memory || byte_count == 0 {
            return Ok(None);
        }
        let Some(last) = address.checked_add(byte_count as u64 - 1) else {
            return Ok(None);
        };
        if address & TLB_PAGE_MASK != last & TLB_PAGE_MASK {
            return Ok(None);
        }
        if let Some(pointer) = self.interpreter_fused_ptr(address, store) {
            return Ok(Some(pointer));
        }

        let access = if store { Access::Store } else { Access::Load };
        let physical = self.translate(bus, address, access)?;
        self.fill_jtlb(bus, address, physical, store);
        Ok(self.interpreter_fused_ptr(address, store))
    }

    /// Fast path for the common architectural case where memory elements and
    /// vector-register elements are both contiguous.  This recognizes only
    /// the RVV addressing mode and its validated configuration, never a guest
    /// PC, surrounding loop, library routine, or benchmark.
    #[inline(always)]
    fn exec_unit_stride_direct<B: Bus>(
        &mut self,
        bus: &mut B,
        load: bool,
        base: u64,
        first_reg: usize,
        start: usize,
        vl: usize,
        element_bytes: usize,
    ) -> Result<usize, Exception> {
        if start >= vl {
            self.vector.vstart = 0;
            return Ok(vl);
        }

        let mut index = start;
        while index < vl {
            let byte_offset = index * element_bytes;
            let address = base.wrapping_add(byte_offset as u64);
            let bytes_to_page = 0x1000usize - (address as usize & 0xfff);
            let page_elements = bytes_to_page / element_bytes;
            if page_elements == 0 {
                // One element itself crosses the boundary; the element path
                // owns its byte-level translation and partial-fault behavior.
                break;
            }
            let run_elements = page_elements.min(vl - index);
            let byte_count = run_elements * element_bytes;
            let memory_pointer = match self.vector_direct_span(bus, address, byte_count, !load) {
                Ok(Some(pointer)) => pointer,
                Ok(None) => break,
                Err(exception) => {
                    self.vector.vstart = index as u64;
                    return Err(exception);
                }
            };
            let vector_offset = first_reg * VLEN_BYTES + byte_offset;

            unsafe {
                // SAFETY: group/vl validation proves the vector range is
                // inside the contiguous register array. `vector_direct_span`
                // proves a live, permission-checked RAM capability for every
                // byte in this page run. Rust's distinct `&mut Cpu` and
                // `&mut Bus` borrows ensure the backing objects do not overlap.
                let vector_pointer = self
                    .vector
                    .regs
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(vector_offset);
                if load {
                    core::ptr::copy_nonoverlapping(
                        core::ptr::with_exposed_provenance::<u8>(memory_pointer),
                        vector_pointer,
                        byte_count,
                    );
                } else {
                    core::ptr::copy_nonoverlapping(
                        vector_pointer,
                        core::ptr::with_exposed_provenance_mut::<u8>(memory_pointer),
                        byte_count,
                    );
                }
            }
            index += run_elements;
        }
        if index == vl {
            self.vector.vstart = 0;
        }
        Ok(index)
    }

    /// Direct-RAM path for an unmasked, single-field constant-stride
    /// transfer whose complete address set lies in one translated page. The
    /// capability is acquired at the first architectural element, preserving
    /// its precise fault point; a fused page row then proves every remaining
    /// (possibly negative- or zero-stride) address in the range.
    fn exec_strided_direct<B: Bus>(
        &mut self,
        bus: &mut B,
        load: bool,
        base: u64,
        first_reg: usize,
        start: usize,
        vl: usize,
        element_bytes: usize,
        stride: u64,
    ) -> Result<bool, Exception> {
        if start >= vl {
            self.vector.vstart = 0;
            return Ok(true);
        }
        if !self.interpreter_fused_memory {
            return Ok(false);
        }

        let step = stride as i64 as i128;
        let first_address = base as i128 + step * start as i128;
        let last_address = base as i128 + step * (vl - 1) as i128;
        let maximum_start = u64::MAX as i128 - (element_bytes - 1) as i128;
        if !(0..=maximum_start).contains(&first_address)
            || !(0..=maximum_start).contains(&last_address)
        {
            return Ok(false);
        }
        let low = first_address.min(last_address) as u64;
        let high = first_address.max(last_address) as u64 + element_bytes as u64 - 1;
        if low & TLB_PAGE_MASK != high & TLB_PAGE_MASK {
            return Ok(false);
        }

        let first_address = first_address as u64;
        let first_pointer = match self.vector_direct_span(bus, first_address, element_bytes, !load)
        {
            Ok(Some(pointer)) => pointer,
            Ok(None) => return Ok(false),
            Err(exception) => {
                self.vector.vstart = start as u64;
                return Err(exception);
            }
        };
        let vector_base = self.vector.regs.as_mut_ptr().cast::<u8>();
        for index in start..vl {
            let pointer_delta = (step * (index - start) as i128) as isize;
            let vector_offset = first_reg * VLEN_BYTES + index * element_bytes;
            unsafe {
                // SAFETY: the fused row proves a live direct-RAM capability
                // for the whole page, the range check keeps every element in
                // that page, and vector group validation bounds the register
                // offset. The CPU and bus are distinct mutable borrows.
                let memory_pointer = core::ptr::with_exposed_provenance_mut::<u8>(first_pointer)
                    .offset(pointer_delta);
                let vector_pointer = vector_base.add(vector_offset);
                if load {
                    core::ptr::copy_nonoverlapping(
                        memory_pointer.cast_const(),
                        vector_pointer,
                        element_bytes,
                    );
                } else {
                    core::ptr::copy_nonoverlapping(vector_pointer, memory_pointer, element_bytes);
                }
            }
        }
        self.vector.vstart = 0;
        Ok(true)
    }

    /// Complete execution path for RVV's ordinary unmasked, single-field,
    /// unit-stride memory form. Keeping this form separate avoids paying for
    /// segmented, indexed, strided, masked, and fault-only-first machinery on
    /// every contiguous transfer; all legality and precise-fault rules remain
    /// architectural and shared across element widths/configurations.
    #[inline(always)]
    fn exec_unit_stride_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        load: bool,
        insn: u32,
        eew: usize,
        base: u64,
        first_reg: usize,
        config: VectorConfig,
    ) -> Result<(), Exception> {
        let span = if eew == config.sew {
            config.lmul().register_span()
        } else {
            config
                .emul_register_span(eew)
                .ok_or(Exception::IllegalInstruction { insn })?
        };
        if (span > 1 && first_reg & (span - 1) != 0) || first_reg + span > VREG_COUNT {
            return Err(Exception::IllegalInstruction { insn });
        }

        let element_bytes = eew / 8;
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        self.vector_dirty();
        let start =
            self.exec_unit_stride_direct(bus, load, base, first_reg, start, vl, element_bytes)?;
        if start == vl {
            return Ok(());
        }

        for index in start..vl {
            let address = base.wrapping_add((index * element_bytes) as u64);
            let result = if load {
                self.vector_load_element(bus, address, eew)
                    .map(|value| self.vector.write_element(first_reg, index, eew, value))
            } else {
                let value = self.vector.read_element(first_reg, index, eew);
                self.vector_store_element(bus, address, eew, value)
            };
            if let Err(exception) = result {
                self.vector.vstart = index as u64;
                return Err(exception);
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    /// Decode and execute the common unit-stride shape without entering the
    /// much larger decoder for every other RVV memory addressing mode.
    #[inline(always)]
    pub(super) fn exec_unit_stride_memory_insn<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: u32,
        load: bool,
    ) -> Result<(), Exception> {
        self.vector_check(insn)?;
        debug_assert!(unit_stride_memory_encoding(insn));
        let eew = vector_width(funct3(insn)).ok_or(Exception::IllegalInstruction { insn })?;
        let config = self
            .vector
            .config()
            .ok_or(Exception::IllegalInstruction { insn })?;
        if self.vector.vstart != 0 && self.vector.vstart >= config.vlmax as u64 {
            return Err(Exception::IllegalInstruction { insn });
        }
        self.exec_unit_stride_memory(bus, load, insn, eew, self.x[rs1(insn)], rd(insn), config)
    }

    fn exec_whole_register_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: u32,
        load: bool,
        eew: usize,
        base: u64,
        first_reg: usize,
        register_count: usize,
    ) -> Result<(), Exception> {
        let evl = register_count * VLEN_BITS / eew;
        if !matches!(register_count, 1 | 2 | 4 | 8)
            || first_reg % register_count != 0
            || first_reg + register_count > VREG_COUNT
            || (!load && eew != 8)
            || self.vector.vstart as usize >= evl
        {
            return Err(Exception::IllegalInstruction { insn });
        }

        let element_bytes = eew / 8;
        let evl = register_count * VLEN_BYTES / element_bytes;
        self.vector_dirty();
        for index in self.vector.vstart as usize..evl {
            let address = base.wrapping_add((index * element_bytes) as u64);
            let result = if load {
                self.vector_load_element(bus, address, eew).map(|value| {
                    self.vector.write_element(first_reg, index, eew, value);
                })
            } else {
                let value = self.vector.read_element(first_reg, index, eew);
                self.vector_store_element(bus, address, eew, value)
            };
            if let Err(exception) = result {
                self.vector.vstart = index as u64;
                return Err(exception);
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    fn exec_mask_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        load: bool,
        base: u64,
        reg: usize,
    ) -> Result<(), Exception> {
        let evl = (self.vector.vl as usize + 7) / 8;
        self.vector_dirty();
        for index in self.vector.vstart as usize..evl {
            let address = base.wrapping_add(index as u64);
            let result = if load {
                self.vector_load_element(bus, address, 8).map(|value| {
                    self.vector.regs[reg][index] = value as u8;
                })
            } else {
                self.vector_store_element(bus, address, 8, self.vector.regs[reg][index] as u64)
            };
            if let Err(exception) = result {
                self.vector.vstart = index as u64;
                return Err(exception);
            }
        }
        self.vector.vstart = 0;
        Ok(())
    }

    pub(super) fn exec_vector_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: u32,
        load: bool,
    ) -> Result<(), Exception> {
        self.vector_check(insn)?;
        let eew = vector_width(funct3(insn)).ok_or(Exception::IllegalInstruction { insn })?;
        let mew = (insn >> 28) & 1;
        let mop = (insn >> 26) & 3;
        let masked = (insn >> 25) & 1 == 0;
        let aux = ((insn >> 20) & 0x1f) as usize;
        let fields = ((insn >> 29) & 7) as usize + 1;
        let first_reg = rd(insn);
        let base = self.x[rs1(insn)];
        if mew != 0 {
            return Err(Exception::IllegalInstruction { insn });
        }

        // Whole-register transfers are independent of the current vl/vtype,
        // which is what makes them suitable for OS context save/restore.
        if mop == 0 && !masked && aux == 8 {
            return self.exec_whole_register_memory(bus, insn, load, eew, base, first_reg, fields);
        }

        let config = self
            .vector
            .config()
            .ok_or(Exception::IllegalInstruction { insn })?;
        if self.vector.vstart != 0 && self.vector.vstart >= config.vlmax as u64 {
            return Err(Exception::IllegalInstruction { insn });
        }

        // Ordinary single-field unit stride (lumop/sumop=00000) is by far the
        // simplest RVV memory form and merits a correspondingly small decoder.
        if mop == 0 && !masked && aux == 0 && fields == 1 {
            return self.exec_unit_stride_memory(bus, load, insn, eew, base, first_reg, config);
        }

        // Mask loads/stores transfer ceil(vl/8) bytes and are unmasked,
        // unit-stride, single-register operations.
        if mop == 0 && !masked && aux == 0x0b && fields == 1 && eew == 8 {
            return self.exec_mask_memory(bus, load, base, first_reg);
        }

        let fault_only_first = load && mop == 0 && aux == 0x10;
        if mop == 0 && aux != 0 && !fault_only_first {
            return Err(Exception::IllegalInstruction { insn });
        }
        if !load && fault_only_first {
            return Err(Exception::IllegalInstruction { insn });
        }

        let indexed = mop == 1 || mop == 3;
        let data_eew = if indexed { config.sew } else { eew };
        let data_ratio = config
            .emul(data_eew)
            .ok_or(Exception::IllegalInstruction { insn })?;
        if !validate_group(first_reg, data_ratio, fields) {
            return Err(Exception::IllegalInstruction { insn });
        }
        let data_span = data_ratio.register_span();

        let index_ratio = if indexed {
            let ratio = config
                .emul(eew)
                .ok_or(Exception::IllegalInstruction { insn })?;
            if !validate_group(aux, ratio, 1) {
                return Err(Exception::IllegalInstruction { insn });
            }
            Some(ratio)
        } else {
            None
        };

        // A masked load cannot overwrite any part of v0 while v0 is still
        // supplying its predicate bits.
        if load && masked && groups_overlap(first_reg, data_ratio, fields, 0, MASK_REGISTER_RATIO) {
            return Err(Exception::IllegalInstruction { insn });
        }

        if indexed {
            let index_ratio = index_ratio.unwrap();
            for field in 0..fields {
                let data_reg = first_reg + field * data_span;
                if load {
                    if !destination_source_overlap_legal(
                        data_reg,
                        data_ratio,
                        data_eew,
                        aux,
                        index_ratio,
                        eew,
                    ) || (fields > 1
                        && groups_overlap(data_reg, data_ratio, 1, aux, index_ratio))
                    {
                        return Err(Exception::IllegalInstruction { insn });
                    }
                } else if mixed_width_sources_overlap(
                    data_reg,
                    data_ratio,
                    data_eew,
                    aux,
                    index_ratio,
                    eew,
                ) {
                    return Err(Exception::IllegalInstruction { insn });
                }
            }
            if masked
                && mixed_width_sources_overlap(aux, index_ratio, eew, 0, MASK_REGISTER_RATIO, 1)
            {
                return Err(Exception::IllegalInstruction { insn });
            }
        }
        if !load && masked {
            for field in 0..fields {
                let data_reg = first_reg + field * data_span;
                if mixed_width_sources_overlap(
                    data_reg,
                    data_ratio,
                    data_eew,
                    0,
                    MASK_REGISTER_RATIO,
                    1,
                ) {
                    return Err(Exception::IllegalInstruction { insn });
                }
            }
        }

        let element_bytes = data_eew / 8;
        let stride = if mop == 2 { self.x[aux] } else { 0 };
        let start = self.vector.vstart as usize;
        let vl = self.vector.vl.min(config.vlmax as u64) as usize;
        self.vector_dirty();

        if mop == 2
            && !masked
            && fields == 1
            && self.exec_strided_direct(
                bus,
                load,
                base,
                first_reg,
                start,
                vl,
                element_bytes,
                stride,
            )?
        {
            return Ok(());
        }

        for index in start..vl {
            if masked && !self.vector.mask_bit(index) {
                continue;
            }
            let element_offset = match mop {
                0 => (index * fields * element_bytes) as u64,
                2 => stride.wrapping_mul(index as u64),
                1 | 3 => self.vector.read_element(aux, index, eew),
                _ => unreachable!(),
            };

            for field in 0..fields {
                let address = base
                    .wrapping_add(element_offset)
                    .wrapping_add((field * element_bytes) as u64);
                let reg = first_reg + field * data_span;
                let result = if load {
                    self.vector_load_element(bus, address, data_eew)
                        .map(|value| self.vector.write_element(reg, index, data_eew, value))
                } else {
                    let value = self.vector.read_element(reg, index, data_eew);
                    self.vector_store_element(bus, address, data_eew, value)
                };

                if let Err(exception) = result {
                    // Fault-only-first loads trap only for element zero. If
                    // element zero is masked off, a fault on the first actual
                    // access at a later index still shortens vl rather than
                    // trapping.
                    if fault_only_first && index != 0 {
                        self.vector.vl = index as u64;
                        self.vector.vstart = 0;
                        return Ok(());
                    }
                    self.vector.vstart = index as u64;
                    return Err(exception);
                }
            }
        }

        self.vector.vstart = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Bus, FlatMemory};

    struct DirectMemory<'a> {
        inner: FlatMemory<'a>,
    }

    impl<'a> DirectMemory<'a> {
        fn new(bytes: &'a mut [u8]) -> Self {
            Self {
                inner: FlatMemory::new(0, bytes),
            }
        }
    }

    impl Bus for DirectMemory<'_> {
        fn read8(&mut self, addr: u64) -> Result<u8, Exception> {
            self.inner.read8(addr)
        }
        fn read16(&mut self, addr: u64) -> Result<u16, Exception> {
            self.inner.read16(addr)
        }
        fn read32(&mut self, addr: u64) -> Result<u32, Exception> {
            self.inner.read32(addr)
        }
        fn read64(&mut self, addr: u64) -> Result<u64, Exception> {
            self.inner.read64(addr)
        }
        fn write8(&mut self, addr: u64, value: u8) -> Result<(), Exception> {
            self.inner.write8(addr, value)
        }
        fn write16(&mut self, addr: u64, value: u16) -> Result<(), Exception> {
            self.inner.write16(addr, value)
        }
        fn write32(&mut self, addr: u64, value: u32) -> Result<(), Exception> {
            self.inner.write32(addr, value)
        }
        fn write64(&mut self, addr: u64, value: u64) -> Result<(), Exception> {
            self.inner.write64(addr, value)
        }
        fn jit_fast_off(&self, va: u64, pa: u64, _store: bool) -> Option<i64> {
            let page = pa & !0xfff;
            (page.checked_add(0x1000)? <= self.inner.mem.len() as u64)
                .then(|| self.inner.mem.as_ptr() as i64 + pa as i64 - va as i64)
        }
    }

    fn vsetvli(d: usize, s1: usize, vtype: u32) -> u32 {
        (vtype << 20) | ((s1 as u32) << 15) | (7 << 12) | ((d as u32) << 7) | 0x57
    }

    fn vector_mem(
        load: bool,
        width: u32,
        nf: usize,
        mop: u32,
        vm: bool,
        aux: usize,
        base: usize,
        reg: usize,
    ) -> u32 {
        (((nf - 1) as u32) << 29)
            | (mop << 26)
            | ((vm as u32) << 25)
            | ((aux as u32) << 20)
            | ((base as u32) << 15)
            | (width << 12)
            | ((reg as u32) << 7)
            | if load { 0x07 } else { 0x27 }
    }

    fn vector_op(
        funct6: u32,
        vm: bool,
        source: usize,
        operand: usize,
        format: u32,
        d: usize,
    ) -> u32 {
        (funct6 << 26)
            | ((vm as u32) << 25)
            | ((source as u32) << 20)
            | ((operand as u32) << 15)
            | (format << 12)
            | ((d as u32) << 7)
            | 0x57
    }

    #[test]
    fn reset_and_vsetvli_obey_vill_and_vl_rules() {
        let mut cpu = Cpu::new();
        assert_eq!(cpu.vector.vtype, VILL);
        assert_eq!(cpu.vector.vl, 0);

        cpu.x[1] = 100;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1
        assert_eq!(cpu.vector.vtype, 0);
        assert_eq!(cpu.vector.vl, 16);
        assert_eq!(cpu.x[2], 16);

        cpu.x[1] = 3;
        cpu.exec_vector_op(vsetvli(2, 1, 0b0_1_000_000)).unwrap(); // e8,m1, tail agnostic
        assert_eq!(cpu.vector.vl, 3);

        cpu.exec_vector_op(vsetvli(2, 1, 4)).unwrap(); // reserved LMUL
        assert_eq!(cpu.vector.vtype, VILL);
        assert_eq!(cpu.vector.vl, 0);
    }

    #[test]
    fn vector_state_is_gated_by_mstatus_vs() {
        let mut cpu = Cpu::new();
        cpu.enable_system(0);
        let insn = vsetvli(1, 0, 0);
        assert_eq!(
            cpu.exec_vector_op(insn),
            Err(Exception::IllegalInstruction { insn })
        );
        assert_eq!(cpu.csr_read(VLENB), None);

        assert!(cpu.csr_write(MSTATUS, MSTATUS_VS & (1 << 9)));
        cpu.exec_vector_op(insn).unwrap();
        assert_eq!(cpu.csr_read(VLENB), Some(16));
        assert_eq!(cpu.sys.as_ref().unwrap().mstatus & MSTATUS_VS, MSTATUS_VS);
    }

    #[test]
    fn unit_stride_load_store_and_precise_restart() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 16;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // vl=16, e8,m1
        cpu.x[3] = 8;
        let load = vector_mem(true, 0, 1, 0, true, 0, 3, 4);

        let mut bytes: Vec<u8> = (0..18).collect();
        let mut bus = FlatMemory::new(0, &mut bytes);
        assert_eq!(
            cpu.exec_vector_memory(&mut bus, load, true),
            Err(Exception::LoadAccessFault { addr: 18 })
        );
        assert_eq!(cpu.vector.vstart, 10);
        assert_eq!(&cpu.vector.regs[4][..10], &(8u8..18).collect::<Vec<_>>());

        // Retry against a now-complete mapping: elements before vstart are
        // retained and execution resumes at the faulting element.
        let mut bytes: Vec<u8> = (0..32).collect();
        let mut bus = FlatMemory::new(0, &mut bytes);
        cpu.exec_vector_memory(&mut bus, load, true).unwrap();
        assert_eq!(cpu.vector.vstart, 0);
        assert_eq!(&cpu.vector.regs[4][..], &(8u8..24).collect::<Vec<_>>());

        cpu.x[3] = 0;
        let store = vector_mem(false, 0, 1, 0, true, 0, 3, 4);
        cpu.exec_vector_memory(&mut bus, store, false).unwrap();
        assert_eq!(&bus.mem[..16], &(8u8..24).collect::<Vec<_>>());
    }

    #[test]
    fn whole_register_transfer_works_with_vill() {
        let mut cpu = Cpu::new();
        assert_eq!(cpu.vector.vtype, VILL);
        cpu.x[1] = 4;
        let load = vector_mem(true, 0, 2, 0, true, 8, 1, 2); // vl2re8.v v2,(x1)
        let mut bytes: Vec<u8> = (0..64).collect();
        let mut bus = FlatMemory::new(0, &mut bytes);
        cpu.exec_vector_memory(&mut bus, load, true).unwrap();
        assert_eq!(&cpu.vector.regs[2][..], &(4u8..20).collect::<Vec<_>>());
        assert_eq!(&cpu.vector.regs[3][..], &(20u8..36).collect::<Vec<_>>());
    }

    #[test]
    fn direct_unit_stride_spans_pages_and_retains_precise_vstart() {
        let mut cpu = Cpu::new();
        cpu.set_interpreter_fused_memory(true);
        cpu.x[1] = 16;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // vl=16, e8,m1
        cpu.x[3] = 4090;
        let load = vector_mem(true, 0, 1, 0, true, 0, 3, 4);
        let store = vector_mem(false, 0, 1, 0, true, 0, 3, 4);

        let mut bytes = vec![0u8; 8192];
        for (index, byte) in bytes[4090..4106].iter_mut().enumerate() {
            *byte = 0x40 + index as u8;
        }
        let mut bus = DirectMemory::new(&mut bytes);
        cpu.exec_vector_memory(&mut bus, load, true).unwrap();
        assert_eq!(&cpu.vector.regs[4], &(0x40u8..0x50).collect::<Vec<_>>()[..]);

        bus.inner.mem[4090..4106].fill(0);
        cpu.exec_vector_memory(&mut bus, store, false).unwrap();
        assert_eq!(
            &bus.inner.mem[4090..4106],
            &(0x40u8..0x50).collect::<Vec<_>>()[..]
        );

        // Only the first page is direct. The first six byte-elements commit,
        // then the next page faults with vstart naming element six.
        let mut short = vec![0u8; 4096];
        short[4090..].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        let mut short_bus = DirectMemory::new(&mut short);
        // Replacing a Bus backing allocation requires the same capability
        // invalidation an emulator performs when replacing/resetting RAM.
        cpu.flush_tlb();
        cpu.x[3] = 4090;
        cpu.vector.regs[4] = [0xaa; VLEN_BYTES];
        assert_eq!(
            cpu.exec_vector_memory(&mut short_bus, load, true),
            Err(Exception::LoadAccessFault { addr: 4096 })
        );
        assert_eq!(cpu.vector.vstart, 6);
        assert_eq!(&cpu.vector.regs[4][..6], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(&cpu.vector.regs[4][6..], &[0xaa; 10]);

        cpu.vector.vstart = 0;
        cpu.vector.regs[4] = [0x5a; VLEN_BYTES];
        assert_eq!(
            cpu.exec_vector_memory(&mut short_bus, store, false),
            Err(Exception::StoreAccessFault { addr: 4096 })
        );
        assert_eq!(cpu.vector.vstart, 6);
        assert_eq!(&short_bus.inner.mem[4090..], &[0x5a; 6]);
    }

    #[test]
    fn optimized_element_access_and_in_place_slide_preserve_group_layout() {
        let mut state = VectorState::default();
        for (reg, index, width, value) in [
            (2, 0, 8, 0x12),
            (2, 1, 16, 0x3456),
            (2, 1, 32, 0x789a_bcde),
            (2, 2, 64, 0x0123_4567_89ab_cdef),
        ] {
            state.write_element(reg, index, width, value);
            assert_eq!(state.read_element(reg, index, width), value);
        }
        // An m2 group flattens across consecutive architectural registers.
        state.write_element(6, 2, 64, 0xfedc_ba98_7654_3210);
        assert_eq!(state.regs[7][..8], 0xfedc_ba98_7654_3210u64.to_le_bytes());
        assert_eq!(state.read_element(6, 2, 64), 0xfedc_ba98_7654_3210);

        let mut cpu = Cpu::new();
        cpu.x[1] = 4;
        // Use e16,mf2 (vlmax=4), the small-group path exercised by LLVM.
        cpu.exec_vector_op(vsetvli(2, 1, 0b1111)).unwrap();
        for (index, value) in [1, 2, 3, 4].into_iter().enumerate() {
            cpu.vector.write_element(8, index, 16, value);
        }
        let slide_down = vector_op(0x0f, true, 8, 1, 3, 8); // vslidedown.vi v8,v8,1
        cpu.exec_vector_op(slide_down).unwrap();
        assert_eq!(
            (0..4)
                .map(|index| cpu.vector.read_element(8, index, 16))
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 0]
        );

        for (index, value) in [1, 2, 3, 4].into_iter().enumerate() {
            cpu.vector.write_element(8, index, 16, value);
        }
        cpu.x[5] = 9;
        let slide1_down = vector_op(0x0f, true, 8, 5, 6, 8);
        cpu.exec_vector_op(slide1_down).unwrap();
        assert_eq!(
            (0..4)
                .map(|index| cpu.vector.read_element(8, index, 16))
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 9]
        );

        for index in 0..4 {
            cpu.vector
                .write_element(8, index, 16, 0x0f0f + index as u64);
            cpu.vector
                .write_element(9, index, 16, 0x3000 + index as u64);
        }
        let bitwise_or = vector_op(0x0a, true, 8, 9, 0, 10);
        cpu.exec_vector_op(bitwise_or).unwrap();
        for index in 0..4 {
            assert_eq!(
                cpu.vector.read_element(10, index, 16),
                (0x0f0f + index as u64) | (0x3000 + index as u64)
            );
        }
    }

    #[test]
    fn fault_only_first_traps_only_at_element_zero_and_trims_later_faults() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 8;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1, vl=8
        cpu.x[3] = 0;
        let load = vector_mem(true, 0, 1, 0, true, 0x10, 3, 4);

        let mut bytes = vec![10, 11, 12, 13];
        let mut bus = FlatMemory::new(0, &mut bytes);
        cpu.exec_vector_memory(&mut bus, load, true).unwrap();
        assert_eq!(cpu.vector.vl, 4);
        assert_eq!(cpu.vector.vstart, 0);
        assert_eq!(&cpu.vector.regs[4][..4], &[10, 11, 12, 13]);

        // A fault on literal element zero still traps and does not alter vl.
        cpu.x[1] = 8;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap();
        cpu.x[3] = 4;
        assert_eq!(
            cpu.exec_vector_memory(&mut bus, load, true),
            Err(Exception::LoadAccessFault { addr: 4 })
        );
        assert_eq!(cpu.vector.vl, 8);
        assert_eq!(cpu.vector.vstart, 0);

        // Masking element zero off does not promote the first actual access
        // to trapping status: a fault at element one trims vl to one.
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap();
        cpu.vector.regs[0] = [0; VLEN_BYTES];
        cpu.vector.regs[0][0] = 1 << 1;
        cpu.x[3] = 3;
        let masked = vector_mem(true, 0, 1, 0, false, 0x10, 3, 4);
        cpu.exec_vector_memory(&mut bus, masked, true).unwrap();
        assert_eq!(cpu.vector.vl, 1);
        assert_eq!(cpu.vector.vstart, 0);

        // Segment FOF uses segment indices for vl/vstart and is mandatory V.
        cpu.x[1] = 8;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap();
        cpu.x[3] = 0;
        let segment = vector_mem(true, 0, 2, 0, true, 0x10, 3, 4);
        cpu.exec_vector_memory(&mut bus, segment, true).unwrap();
        assert_eq!(cpu.vector.vl, 2);
        assert_eq!(&cpu.vector.regs[4][..2], &[10, 12]);
        assert_eq!(&cpu.vector.regs[5][..2], &[11, 13]);
    }

    #[test]
    fn mixed_width_overlap_obeys_rvv_low_and_high_end_rules() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 16;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1
        for index in 0..16 {
            cpu.vector.write_element(17, index, 8, index as u64 + 1);
            cpu.vector.write_element(12, index, 8, index as u64 + 20);
        }

        // Widening may overlap a full-register narrow source at the high end.
        let widening = vector_op(0x30, true, 17, 12, 2, 16); // vwaddu.vv
        cpu.exec_vector_op(widening).unwrap();
        for index in 0..16 {
            assert_eq!(
                cpu.vector.read_element(16, index, 16),
                (index as u64 + 1) + (index as u64 + 20)
            );
        }

        // The same source at the low end, or a fractional-EMUL source, is
        // reserved and therefore traps as an illegal instruction here.
        let low_end = vector_op(0x30, true, 16, 12, 2, 16);
        assert_eq!(
            cpu.exec_vector_op(low_end),
            Err(Exception::IllegalInstruction { insn: low_end })
        );
        cpu.x[1] = 8;
        cpu.exec_vector_op(vsetvli(2, 1, 0b111)).unwrap(); // e8,mf2
        let fractional = vector_op(0x30, true, 16, 12, 2, 16);
        assert_eq!(
            cpu.exec_vector_op(fractional),
            Err(Exception::IllegalInstruction { insn: fractional })
        );

        // Narrowing is the mirror image: sharing the low register is legal.
        cpu.x[1] = 16;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap();
        let expected: Vec<u64> = (0..16).map(|i| (0x100 + i * 17) >> 3).collect();
        for (index, value) in (0..16).map(|i| 0x100 + i * 17).enumerate() {
            cpu.vector.write_element(16, index, 16, value);
        }
        let narrowing = vector_op(0x2c, true, 16, 3, 3, 16); // vnsrl.wi
        cpu.exec_vector_op(narrowing).unwrap();
        for (index, expected) in expected.into_iter().enumerate() {
            assert_eq!(cpu.vector.read_element(16, index, 8), expected & 0xff);
        }
        let wrong_low_end = vector_op(0x2c, true, 16, 3, 3, 17);
        assert_eq!(
            cpu.exec_vector_op(wrong_low_end),
            Err(Exception::IllegalInstruction {
                insn: wrong_low_end
            })
        );
    }

    #[test]
    fn mask_sources_overwrite_destinations_and_wide_accumulators_are_legal_only_as_specified() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 16;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1
        for index in 0..16 {
            cpu.vector.write_element(8, index, 8, index as u64);
            cpu.vector.write_element(12, index, 8, index as u64);
        }

        // A narrowing mask result can share the low register of a source.
        let compare_overlap = vector_op(0x18, true, 8, 12, 0, 8);
        cpu.exec_vector_op(compare_overlap).unwrap();
        assert_eq!(
            u16::from_le_bytes(cpu.vector.regs[8][..2].try_into().unwrap()),
            0xffff
        );

        // A masked compare may overwrite v0 while consuming its old bits.
        for index in 0..16 {
            cpu.vector.write_element(8, index, 8, index as u64);
            cpu.vector.write_element(12, index, 8, index as u64);
        }
        cpu.vector.regs[0] = [0; VLEN_BYTES];
        cpu.vector.regs[0][0] = 0b1010_1010;
        let compare_v0 = vector_op(0x18, false, 8, 12, 0, 0);
        cpu.exec_vector_op(compare_v0).unwrap();
        assert_eq!(cpu.vector.regs[0][0], 0b1010_1010);

        // A data source cannot simultaneously be interpreted as the one-bit
        // v0 predicate, and a wide accumulator cannot also be a narrow input.
        let data_is_mask = vector_op(0x00, false, 0, 12, 0, 16);
        assert_eq!(
            cpu.exec_vector_op(data_is_mask),
            Err(Exception::IllegalInstruction { insn: data_is_mask })
        );
        let wide_accumulator_overlap = vector_op(0x3c, true, 17, 12, 2, 16);
        assert_eq!(
            cpu.exec_vector_op(wide_accumulator_overlap),
            Err(Exception::IllegalInstruction {
                insn: wide_accumulator_overlap
            })
        );
        let mixed_sources = vector_op(0x34, true, 16, 16, 2, 20);
        assert_eq!(
            cpu.exec_vector_op(mixed_sources),
            Err(Exception::IllegalInstruction {
                insn: mixed_sources
            })
        );
    }

    #[test]
    fn special_vstart_vl_zero_and_indexed_memory_rules_are_precise() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 2;
        cpu.exec_vector_op(vsetvli(2, 1, 0b11_000)).unwrap(); // e64,m1, vl=2

        let whole_move = vector_op(0x27, true, 8, 0, 3, 16); // vmv1r.v
        cpu.vector.vstart = 2; // evl is two e64 elements
        assert_eq!(
            cpu.exec_vector_op(whole_move),
            Err(Exception::IllegalInstruction { insn: whole_move })
        );
        assert_eq!(cpu.vector.vstart, 2);

        let mut bytes = vec![0u8; 64];
        let mut bus = FlatMemory::new(0, &mut bytes);
        cpu.x[3] = 0;
        let whole_load = vector_mem(true, 7, 1, 0, true, 8, 3, 16);
        assert_eq!(
            cpu.exec_vector_memory(&mut bus, whole_load, true),
            Err(Exception::IllegalInstruction { insn: whole_load })
        );
        assert_eq!(cpu.vector.vstart, 2);

        // Reductions with vl=0 do not copy the scalar seed to the result.
        cpu.vector.vstart = 0;
        cpu.x[1] = 0;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1, vl=0
        cpu.vector.regs[16] = [0xa5; VLEN_BYTES];
        let reduction = vector_op(0x00, true, 8, 12, 2, 16);
        cpu.exec_vector_op(reduction).unwrap();
        assert_eq!(cpu.vector.regs[16], [0xa5; VLEN_BYTES]);

        // A single indexed load may use one group as both equal-width index
        // and destination; reading each index precedes writing that element.
        cpu.x[1] = 4;
        cpu.exec_vector_op(vsetvli(2, 1, 0b10_000)).unwrap(); // e32,m1
        for (index, offset) in [0, 4, 8, 12].into_iter().enumerate() {
            cpu.vector.write_element(8, index, 32, offset);
        }
        for (index, value) in [101u32, 202, 303, 404].into_iter().enumerate() {
            bus.mem[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let indexed = vector_mem(true, 6, 1, 1, true, 8, 3, 8);
        cpu.exec_vector_memory(&mut bus, indexed, true).unwrap();
        assert_eq!(
            (0..4)
                .map(|index| cpu.vector.read_element(8, index, 32))
                .collect::<Vec<_>>(),
            vec![101, 202, 303, 404]
        );

        let indexed_segment = vector_mem(true, 6, 2, 1, true, 8, 3, 8);
        assert_eq!(
            cpu.exec_vector_memory(&mut bus, indexed_segment, true),
            Err(Exception::IllegalInstruction {
                insn: indexed_segment
            })
        );
        let masked_store_v0 = vector_mem(false, 0, 1, 0, false, 0, 3, 0);
        assert_eq!(
            cpu.exec_vector_memory(&mut bus, masked_store_v0, false),
            Err(Exception::IllegalInstruction {
                insn: masked_store_v0
            })
        );
    }

    #[test]
    fn fixed_point_saturation_sets_the_architectural_sticky_flag() {
        let mut cpu = Cpu::new();
        cpu.x[1] = 1;
        cpu.exec_vector_op(vsetvli(2, 1, 0)).unwrap(); // e8,m1, vl=1
        cpu.vector.write_element(8, 0, 8, 0x80);
        cpu.vector.write_element(12, 0, 8, 0x80);
        let multiply = vector_op(0x27, true, 8, 12, 0, 16); // vsmul.vv
        cpu.exec_vector_op(multiply).unwrap();
        assert_eq!(cpu.vector.read_element(16, 0, 8), 0x7f);
        assert!(cpu.vector.vxsat);

        // A later nonsaturating operation cannot clear the sticky bit.
        cpu.vector.write_element(8, 0, 8, 1);
        cpu.vector.write_element(12, 0, 8, 1);
        cpu.exec_vector_op(multiply).unwrap();
        assert!(cpu.vector.vxsat);
    }
}
