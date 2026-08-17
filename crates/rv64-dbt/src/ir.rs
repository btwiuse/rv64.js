//! Typed SSA and ordered effects used by the first compiler tier.
//!
//! Architectural register forwarding happens while lifting: a register read
//! becomes one `ReadX` value and later reads see the latest SSA definition.
//! Stores to architectural registers are represented by `Region::outputs`,
//! making overwritten guest writes naturally dead. Guest-memory loads remain
//! value-producing nodes while stores carry an explicit position in the SSA
//! stream. Both retain a precise pre-instruction architectural side exit.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValueId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType {
    I32,
    I64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    I64Add,
    I64Sub,
    I64Mul,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrU,
    I64ShrS,
    I32Add,
    I32Sub,
    I32Mul,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrU,
    I32ShrS,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LtU,
    I64GeS,
    I64GeU,
}

/// RV64 division/remainder semantics. These are not plain WebAssembly binary
/// operators: Wasm traps on a zero divisor (and signed division overflow),
/// while RISC-V returns architecturally defined values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DivideOp {
    I64DivS,
    I64DivU,
    I64RemS,
    I64RemU,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
}

impl DivideOp {
    pub const fn value_type(self) -> ValueType {
        match self {
            Self::I64DivS | Self::I64DivU | Self::I64RemS | Self::I64RemU => ValueType::I64,
            Self::I32DivS | Self::I32DivU | Self::I32RemS | Self::I32RemU => ValueType::I32,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::I64DivS | Self::I64RemS | Self::I32DivS | Self::I32RemS
        )
    }

    pub const fn is_remainder(self) -> bool {
        matches!(
            self,
            Self::I64RemS | Self::I64RemU | Self::I32RemS | Self::I32RemU
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MulHighKind {
    SignedSigned,
    SignedUnsigned,
    UnsignedUnsigned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum ExactFpOp {
    Add32,
    Sub32,
    Mul32,
    Div32,
    Add64,
    Sub64,
    Mul64,
    Div64,
    Eq32,
    Lt32,
    Le32,
    Eq64,
    Lt64,
    Le64,
    Sqrt32,
    Sqrt64,
    Min32,
    Max32,
    Min64,
    Max64,
    Cvt32From64,
    Cvt64From32,
    CvtI32From32,
    CvtU32From32,
    CvtI64From32,
    CvtU64From32,
    CvtI32From64,
    CvtU32From64,
    CvtI64From64,
    CvtU64From64,
    Cvt32FromI32,
    Cvt32FromU32,
    Cvt32FromI64,
    Cvt32FromU64,
    Cvt64FromI32,
    Cvt64FromU32,
    Cvt64FromI64,
    Cvt64FromU64,
    Class32,
    Class64,
    Fma32,
    Fma64,
}

impl ExactFpOp {
    pub const fn helper_code(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadKind {
    I8S,
    I16S,
    I32S,
    I64,
    I8U,
    I16U,
    I32U,
}

impl LoadKind {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::I8S | Self::I8U => 1,
            Self::I16S | Self::I16U => 2,
            Self::I32S | Self::I32U => 4,
            Self::I64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreKind {
    I8,
    I16,
    I32,
    I64,
}

impl StoreKind {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::I8 => 1,
            Self::I16 => 2,
            Self::I32 => 4,
            Self::I64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationOp {
    LoadReserved,
    StoreConditional,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideExit {
    pub guest_pc: u64,
    pub retired: u32,
    pub outputs: Vec<(u8, ValueId)>,
    pub f_outputs: Vec<(u8, ValueId)>,
    pub fcsr_output: Option<ValueId>,
}

/// Lane-wise RVV operations that have an exact WebAssembly SIMD128
/// representation for at least one SEW. The emitter still guards the dynamic
/// vtype/vl/vstart and register-group legality before selecting this path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorLaneOp {
    Add,
    Sub,
    ReverseSub,
    MinUnsigned,
    MinSigned,
    MaxUnsigned,
    MaxSigned,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRightUnsigned,
    ShiftRightSigned,
    Multiply,
    Move,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorFloatSignOp {
    Copy,
    Negate,
    Xor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorCompareOp {
    Equal,
    NotEqual,
    LessUnsigned,
    LessSigned,
    LessEqualUnsigned,
    LessEqualSigned,
    GreaterUnsigned,
    GreaterSigned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorReductionOp {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorMaskOp {
    AndNot,
    And,
    Or,
    Xor,
    OrNot,
    Nand,
    Nor,
    Xnor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorOperand {
    Vector(u8),
    ScalarX(u8),
    Immediate(i8),
}

/// A conservative direct-SIMD candidate derived only from the architectural
/// instruction encoding. Runtime guards decide whether the current vector
/// configuration is exactly representable; otherwise the typed helper remains
/// the authoritative fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorDirect {
    /// Immediate vector configuration with all architectural results resolved
    /// from the instruction encoding and this machine's fixed VLEN/ELEN.
    ConfigImmediate {
        destination: u8,
        vtype: u64,
        vl: u64,
    },
    /// `vsetvli x0,x0,vtype`, specialized only when a runtime guard proves
    /// that both the old and new configurations have this full VLMAX.
    ConfigRetainFull {
        vtype: u64,
        vlmax: u64,
    },
    Lane {
        op: VectorLaneOp,
        masked: bool,
        destination: u8,
        source2: Option<u8>,
        operand: VectorOperand,
    },
    UnitStride {
        load: bool,
        masked: bool,
        width: u8,
        register: u8,
        base: u8,
    },
    Strided {
        load: bool,
        masked: bool,
        width: u8,
        register: u8,
        base: u8,
        stride: u8,
    },
    WholeRegisterMemory {
        load: bool,
        register: u8,
        base: u8,
        registers: u8,
    },
    WholeRegisterMove {
        destination: u8,
        source: u8,
        registers: u8,
    },
    ScalarExtract {
        destination: u8,
        source: u8,
    },
    ScalarInsert {
        destination: u8,
        source: u8,
    },
    SlideImmediate {
        up: bool,
        destination: u8,
        source: u8,
        offset: u8,
    },
    SlideOne {
        up: bool,
        destination: u8,
        source: u8,
        scalar: u8,
    },
    GatherImmediate {
        destination: u8,
        source: u8,
        index: u8,
    },
    GatherVector {
        destination: u8,
        source: u8,
        indices: u8,
    },
    /// Write each active element's architectural index (`vid.v`).
    Index {
        destination: u8,
    },
    /// Unmasked bitwise integer reduction into element zero of `destination`.
    Reduction {
        op: VectorReductionOp,
        destination: u8,
        source: u8,
        seed: u8,
    },
    /// Boolean operation over packed architectural mask bits.
    MaskLogic {
        op: VectorMaskOp,
        destination: u8,
        source2: u8,
        source1: u8,
    },
    WidenAddSub {
        signed: bool,
        subtract: bool,
        wide_left: bool,
        destination: u8,
        source2: u8,
        operand: VectorOperand,
    },
    WidenMultiplyAccumulate {
        source2_signed: bool,
        operand_signed: bool,
        destination: u8,
        source2: u8,
        operand: VectorOperand,
    },
    FloatSign {
        op: VectorFloatSignOp,
        destination: u8,
        source2: u8,
        source1: u8,
    },
    FloatBroadcast {
        destination: u8,
        source: u8,
    },
    FloatScalarInsert {
        destination: u8,
        source: u8,
    },
    FloatScalarExtract {
        destination: u8,
        source: u8,
    },
    FloatSlideOne {
        up: bool,
        destination: u8,
        source: u8,
        scalar: u8,
    },
    Compare {
        op: VectorCompareOp,
        destination: u8,
        source2: u8,
        operand: VectorOperand,
    },
}

fn decode_vector_lane(
    funct6: u32,
    destination: u8,
    format: u32,
    source: u8,
    source2: u8,
    masked: bool,
) -> Option<VectorDirect> {
    let (op, allowed_formats) = match funct6 {
        0x00 => (VectorLaneOp::Add, 0b0001_1001),
        0x02 => (VectorLaneOp::Sub, 0b0001_0001),
        0x03 => (VectorLaneOp::ReverseSub, 0b0001_1000),
        0x04 => (VectorLaneOp::MinUnsigned, 0b0001_0001),
        0x05 => (VectorLaneOp::MinSigned, 0b0001_0001),
        0x06 => (VectorLaneOp::MaxUnsigned, 0b0001_0001),
        0x07 => (VectorLaneOp::MaxSigned, 0b0001_0001),
        0x09 => (VectorLaneOp::And, 0b0001_1001),
        0x0a => (VectorLaneOp::Or, 0b0001_1001),
        0x0b => (VectorLaneOp::Xor, 0b0001_1001),
        // vmul uses the OPMVV/OPMVX formats rather than OPIVV/OPIVX.
        0x25 if matches!(format, 2 | 6) => (VectorLaneOp::Multiply, 0b0100_0100),
        // Wasm SIMD shifts have one scalar shift amount. Per-lane VV shifts
        // deliberately retain the helper fallback.
        0x25 => (VectorLaneOp::ShiftLeft, 0b0001_1000),
        0x28 => (VectorLaneOp::ShiftRightUnsigned, 0b0001_1000),
        0x29 => (VectorLaneOp::ShiftRightSigned, 0b0001_1000),
        // The vm=0 encoding in this slot is vmerge, not a masked move.
        0x17 if !masked && source2 == 0 => (VectorLaneOp::Move, 0b0001_1001),
        _ => return None,
    };
    if allowed_formats & (1 << format) == 0 {
        return None;
    }

    let operand = match format {
        0 | 2 => VectorOperand::Vector(source),
        3 if matches!(
            op,
            VectorLaneOp::ShiftLeft
                | VectorLaneOp::ShiftRightUnsigned
                | VectorLaneOp::ShiftRightSigned
        ) =>
        {
            VectorOperand::Immediate(source as i8)
        }
        3 => VectorOperand::Immediate(((source << 3) as i8) >> 3),
        4 | 6 => VectorOperand::ScalarX(source),
        _ => return None,
    };
    Some(VectorDirect::Lane {
        op,
        masked,
        destination,
        source2: (op != VectorLaneOp::Move).then_some(source2),
        operand,
    })
}

impl VectorDirect {
    pub fn decode(insn: u32) -> Option<Self> {
        let opcode = insn & 0x7f;
        let destination = ((insn >> 7) & 0x1f) as u8;
        let format = (insn >> 12) & 7;
        let source = ((insn >> 15) & 0x1f) as u8;
        let source2 = ((insn >> 20) & 0x1f) as u8;
        let unmasked = insn & (1 << 25) != 0;

        if opcode == 0x57
            && format == 7
            && (insn >> 30 == 0b11 || (insn >> 31 == 0 && source == 0 && destination != 0))
        {
            let immediate_avl = insn >> 30 == 0b11;
            let raw_type = u64::from((insn >> 20) & if immediate_avl { 0x3ff } else { 0x7ff });
            // vsetivli carries AVL in the rs1 field. For vsetvli rd,x0,
            // the ratified AVL table requests VLMAX exactly.
            let avl = if immediate_avl {
                u64::from(source)
            } else {
                u64::MAX
            };
            let (vtype, vl) = decode_immediate_vector_config(raw_type, avl);
            return Some(Self::ConfigImmediate {
                destination,
                vtype,
                vl,
            });
        }

        if opcode == 0x57 && format == 7 && insn >> 31 == 0 && source == 0 && destination == 0 {
            let raw_type = u64::from((insn >> 20) & 0x7ff);
            let (vtype, vlmax) = decode_immediate_vector_config(raw_type, u64::MAX);
            if vtype >> 63 == 0 {
                return Some(Self::ConfigRetainFull { vtype, vlmax });
            }
        }

        if matches!(opcode, 0x07 | 0x27) {
            let width = match format {
                0 => 8,
                5 => 16,
                6 => 32,
                7 => 64,
                _ => return None,
            };
            let fields = ((insn >> 29) & 7) as u8 + 1;
            let mew = (insn >> 28) & 1;
            let mop = (insn >> 26) & 3;
            let aux = (insn >> 20) & 0x1f;
            if mew == 0 && mop == 0 && aux == 0 && fields == 1 {
                return Some(Self::UnitStride {
                    load: opcode == 0x07,
                    masked: !unmasked,
                    width,
                    register: destination,
                    base: source,
                });
            }
            if mew == 0 && mop == 2 && fields == 1 {
                return Some(Self::Strided {
                    load: opcode == 0x07,
                    masked: !unmasked,
                    width,
                    register: destination,
                    base: source,
                    stride: aux as u8,
                });
            }
            if unmasked
                && mew == 0
                && mop == 0
                && aux == 8
                && fields.is_power_of_two()
                && (opcode == 0x07 || width == 8)
            {
                return Some(Self::WholeRegisterMemory {
                    load: opcode == 0x07,
                    register: destination,
                    base: source,
                    registers: fields,
                });
            }
            return None;
        }

        if opcode != 0x57 || format == 7 {
            return None;
        }
        let funct6 = insn >> 26;
        if !unmasked {
            return decode_vector_lane(funct6, destination, format, source, source2, true);
        }
        if format == 1 {
            if funct6 == 0x10 && source == 0 {
                return Some(Self::FloatScalarExtract {
                    destination,
                    source: source2,
                });
            }
            let op = match funct6 {
                0x08 => VectorFloatSignOp::Copy,
                0x09 => VectorFloatSignOp::Negate,
                0x0a => VectorFloatSignOp::Xor,
                _ => return None,
            };
            return Some(Self::FloatSign {
                op,
                destination,
                source2,
                source1: source,
            });
        }
        if format == 5 {
            if funct6 == 0x10 && source2 == 0 {
                return Some(Self::FloatScalarInsert {
                    destination,
                    source,
                });
            }
            if funct6 == 0x17 && source2 == 0 {
                return Some(Self::FloatBroadcast {
                    destination,
                    source,
                });
            }
            if matches!(funct6, 0x0e | 0x0f) {
                return Some(Self::FloatSlideOne {
                    up: funct6 == 0x0e,
                    destination,
                    source: source2,
                    scalar: source,
                });
            }
            return None;
        }
        if funct6 == 0x10 && format == 6 && source2 == 0 {
            return Some(Self::ScalarInsert {
                destination,
                source,
            });
        }
        if funct6 == 0x10 && format == 2 && source == 0 {
            return Some(Self::ScalarExtract {
                destination,
                source: source2,
            });
        }
        if funct6 == 0x27 && format == 3 {
            let registers = source + 1;
            return registers
                .is_power_of_two()
                .then_some(Self::WholeRegisterMove {
                    destination,
                    source: source2,
                    registers,
                });
        }
        if matches!(funct6, 0x0e | 0x0f) && format == 3 {
            return Some(Self::SlideImmediate {
                up: funct6 == 0x0e,
                destination,
                source: source2,
                offset: source,
            });
        }
        if matches!(funct6, 0x0e | 0x0f) && format == 6 {
            return Some(Self::SlideOne {
                up: funct6 == 0x0e,
                destination,
                source: source2,
                scalar: source,
            });
        }
        if funct6 == 0x0c && format == 3 {
            return Some(Self::GatherImmediate {
                destination,
                source: source2,
                index: source,
            });
        }
        if funct6 == 0x0c && format == 0 {
            return Some(Self::GatherVector {
                destination,
                source: source2,
                indices: source,
            });
        }
        if funct6 == 0x14 && format == 2 && source2 == 0 && source == 0x11 {
            return Some(Self::Index { destination });
        }
        if format == 2 {
            let op = match funct6 {
                0x18 => Some(VectorMaskOp::AndNot),
                0x19 => Some(VectorMaskOp::And),
                0x1a => Some(VectorMaskOp::Or),
                0x1b => Some(VectorMaskOp::Xor),
                0x1c => Some(VectorMaskOp::OrNot),
                0x1d => Some(VectorMaskOp::Nand),
                0x1e => Some(VectorMaskOp::Nor),
                0x1f => Some(VectorMaskOp::Xnor),
                _ => None,
            };
            if let Some(op) = op {
                return Some(Self::MaskLogic {
                    op,
                    destination,
                    source2,
                    source1: source,
                });
            }
            let op = match funct6 {
                0x01 => Some(VectorReductionOp::And),
                0x02 => Some(VectorReductionOp::Or),
                0x03 => Some(VectorReductionOp::Xor),
                _ => None,
            };
            if let Some(op) = op {
                return Some(Self::Reduction {
                    op,
                    destination,
                    source: source2,
                    seed: source,
                });
            }
        }
        if matches!(format, 2 | 6) && matches!(funct6, 0x30..=0x37) {
            return Some(Self::WidenAddSub {
                signed: funct6 & 1 != 0,
                subtract: funct6 & 2 != 0,
                wide_left: funct6 >= 0x34,
                destination,
                source2,
                operand: if format == 2 {
                    VectorOperand::Vector(source)
                } else {
                    VectorOperand::ScalarX(source)
                },
            });
        }
        if matches!(format, 2 | 6)
            && matches!(funct6, 0x3c..=0x3f)
            && !(funct6 == 0x3e && format == 2)
        {
            let (source2_signed, operand_signed) = match funct6 {
                0x3c => (false, false),
                0x3d => (true, true),
                0x3e => (true, false),
                0x3f => (false, true),
                _ => unreachable!(),
            };
            return Some(Self::WidenMultiplyAccumulate {
                source2_signed,
                operand_signed,
                destination,
                source2,
                operand: if format == 2 {
                    VectorOperand::Vector(source)
                } else {
                    VectorOperand::ScalarX(source)
                },
            });
        }
        let compare = match funct6 {
            0x18 => Some((VectorCompareOp::Equal, 0b0001_1001)),
            0x19 => Some((VectorCompareOp::NotEqual, 0b0001_1001)),
            0x1a => Some((VectorCompareOp::LessUnsigned, 0b0001_0001)),
            0x1b => Some((VectorCompareOp::LessSigned, 0b0001_0001)),
            0x1c => Some((VectorCompareOp::LessEqualUnsigned, 0b0001_1001)),
            0x1d => Some((VectorCompareOp::LessEqualSigned, 0b0001_1001)),
            0x1e => Some((VectorCompareOp::GreaterUnsigned, 0b0001_1000)),
            0x1f => Some((VectorCompareOp::GreaterSigned, 0b0001_1000)),
            _ => None,
        };
        if let Some((op, allowed_formats)) = compare {
            if allowed_formats & (1 << format) == 0 {
                return None;
            }
            let operand = match format {
                0 => VectorOperand::Vector(source),
                3 => VectorOperand::Immediate(((source << 3) as i8) >> 3),
                4 => VectorOperand::ScalarX(source),
                _ => return None,
            };
            return Some(Self::Compare {
                op,
                destination,
                source2,
                operand,
            });
        }
        decode_vector_lane(funct6, destination, format, source, source2, false)
    }

    pub const fn uses_memory(self) -> bool {
        matches!(
            self,
            Self::UnitStride { .. } | Self::Strided { .. } | Self::WholeRegisterMemory { .. }
        )
    }
}

fn decode_immediate_vector_config(raw_type: u64, avl: u64) -> (u64, u64) {
    const VILL: u64 = 1 << 63;
    if raw_type >> 8 != 0 {
        return (VILL, 0);
    }
    let lmul_exp = match raw_type & 7 {
        0 => 0i32,
        1 => 1,
        2 => 2,
        3 => 3,
        5 => -3,
        6 => -2,
        7 => -1,
        _ => return (VILL, 0),
    };
    let vsew = i32::try_from((raw_type >> 3) & 7).expect("three-bit vsew");
    if vsew > 3 {
        return (VILL, 0);
    }
    let sew_log = 3 + vsew;
    if sew_log > 6 + lmul_exp {
        return (VILL, 0);
    }
    let vlmax_log = 7 + lmul_exp - sew_log;
    if vlmax_log < 0 {
        return (VILL, 0);
    }
    let vlmax = 1u64 << vlmax_log;
    (raw_type & 0xff, avl.min(vlmax))
}

impl SideExit {
    fn for_each_value(&self, mut f: impl FnMut(ValueId)) {
        for &(_, value) in &self.outputs {
            f(value);
        }
        for &(_, value) in &self.f_outputs {
            f(value);
        }
        if let Some(value) = self.fcsr_output {
            f(value);
        }
    }

    fn remap(&self, map: &[Option<ValueId>]) -> Self {
        Self {
            guest_pc: self.guest_pc,
            retired: self.retired,
            outputs: self
                .outputs
                .iter()
                .map(|&(reg, value)| {
                    (
                        reg,
                        map[value.0].expect("side-exit output must remain live"),
                    )
                })
                .collect(),
            f_outputs: self
                .f_outputs
                .iter()
                .map(|&(reg, value)| {
                    (
                        reg,
                        map[value.0].expect("side-exit FP output must remain live"),
                    )
                })
                .collect(),
            fcsr_output: self
                .fcsr_output
                .map(|value| map[value.0].expect("side-exit fcsr output must remain live")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Store {
        /// Emit before the value at this index, or after all values when equal
        /// to `Region::values.len()`.
        position: usize,
        address: ValueId,
        value: ValueId,
        kind: StoreKind,
        /// SC supplies a success condition; a failed SC performs no memory
        /// access. Ordinary stores and AMOs use `None`.
        condition: Option<ValueId>,
        exit: SideExit,
    },
    /// Exit after a guest control instruction when `condition` is true. The
    /// normal trace path continues at this effect's position.
    Guard {
        position: usize,
        condition: ValueId,
        exit: SideExit,
    },
    /// Continue along a profiled indirect edge only when the computed target
    /// still matches. A mismatch exits to the computed `target`, not to a
    /// compiler constant, after retiring the indirect control instruction.
    GuardTarget {
        position: usize,
        target: ValueId,
        expected: u64,
        exit: SideExit,
    },
    /// Full-system floating-point availability check at the exact guest
    /// instruction boundary. `dirty` mirrors the interpreter's fp_dirty call;
    /// stores check FS but do not change Clean/Initial state.
    FpState {
        position: usize,
        dirty: bool,
        exit: SideExit,
    },
    /// Full-system vector availability check at an exact architectural
    /// boundary. Read-only vector CSR lowering uses this without dirtying VS;
    /// ordinary vector instructions retain the complete `Vector` effect below.
    VectorState { position: usize, exit: SideExit },
    /// One complete architectural RVV instruction. The backend publishes the
    /// pre-instruction scalar state, calls the concrete machine's typed vector
    /// helper, and invalidates cached x/f/fcsr values on success.
    Vector {
        position: usize,
        insn: u32,
        direct: Option<VectorDirect>,
        fallthrough: u64,
        exit: SideExit,
    },
}

impl BinaryOp {
    pub fn input_type(self) -> ValueType {
        match self {
            Self::I32Add
            | Self::I32Sub
            | Self::I32Mul
            | Self::I32And
            | Self::I32Or
            | Self::I32Xor
            | Self::I32Shl
            | Self::I32ShrU
            | Self::I32ShrS => ValueType::I32,
            _ => ValueType::I64,
        }
    }

    pub fn result_type(self) -> ValueType {
        match self {
            Self::I64Eq
            | Self::I64Ne
            | Self::I64LtS
            | Self::I64LtU
            | Self::I64GeS
            | Self::I64GeU => ValueType::I32,
            op => op.input_type(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
    ConstI32(i32),
    ConstI64(i64),
    /// An architectural address derived from the current instruction PC
    /// (AUIPC, direct control targets/links, and precise static exits).  It is
    /// a plain absolute constant for ordinary modules, but retaining the
    /// provenance lets a position-independent page module encode it as
    /// `guest_base + offset` without guessing which integer constants happen
    /// to resemble code addresses.
    GuestPc(u64),
    ReadX(u8),
    ReadF(u8),
    ReadFcsr,
    Binary {
        op: BinaryOp,
        lhs: ValueId,
        rhs: ValueId,
    },
    Divide {
        op: DivideOp,
        lhs: ValueId,
        rhs: ValueId,
    },
    WrapI64ToI32(ValueId),
    ExtendI32S(ValueId),
    ExtendI32U(ValueId),
    SelectI64 {
        condition: ValueId,
        if_true: ValueId,
        if_false: ValueId,
    },
    Load {
        address: ValueId,
        kind: LoadKind,
        exit: SideExit,
    },
    /// Exact software-FP helper call. The helper updates the shared fcsr cell;
    /// `ReloadFcsr` captures that second result into SSA immediately after it.
    ExactFp {
        op: ExactFpOp,
        lhs: ValueId,
        rhs: ValueId,
        third: ValueId,
        rm: ValueId,
        fcsr: ValueId,
        exit: SideExit,
    },
    /// User-mode LR/SC reservation transition. The helper returns i32 true
    /// when a store-conditional owns the matching reservation.
    Reservation {
        op: ReservationOp,
        address: ValueId,
    },
    ReloadFcsr(ValueId),
}

impl Op {
    pub fn for_each_operand(&self, mut f: impl FnMut(ValueId)) {
        match *self {
            Self::ConstI32(_)
            | Self::ConstI64(_)
            | Self::GuestPc(_)
            | Self::ReadX(_)
            | Self::ReadF(_)
            | Self::ReadFcsr => {}
            Self::Binary { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Self::Divide { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Self::WrapI64ToI32(value) | Self::ExtendI32S(value) | Self::ExtendI32U(value) => {
                f(value)
            }
            Self::SelectI64 {
                condition,
                if_true,
                if_false,
            } => {
                f(condition);
                f(if_true);
                f(if_false);
            }
            Self::Load {
                address, ref exit, ..
            } => {
                f(address);
                exit.for_each_value(f);
            }
            Self::ExactFp {
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                ref exit,
                ..
            } => {
                f(lhs);
                f(rhs);
                f(third);
                f(rm);
                f(fcsr);
                exit.for_each_value(f);
            }
            Self::Reservation { address, .. } => f(address),
            Self::ReloadFcsr(value) => f(value),
        }
    }

    fn remap(&self, map: &[Option<ValueId>]) -> Self {
        let get = |id: ValueId| map[id.0].expect("live operand must have a remapped value");
        match *self {
            Self::ConstI32(v) => Self::ConstI32(v),
            Self::ConstI64(v) => Self::ConstI64(v),
            Self::GuestPc(v) => Self::GuestPc(v),
            Self::ReadX(reg) => Self::ReadX(reg),
            Self::ReadF(reg) => Self::ReadF(reg),
            Self::ReadFcsr => Self::ReadFcsr,
            Self::Binary { op, lhs, rhs } => Self::Binary {
                op,
                lhs: get(lhs),
                rhs: get(rhs),
            },
            Self::Divide { op, lhs, rhs } => Self::Divide {
                op,
                lhs: get(lhs),
                rhs: get(rhs),
            },
            Self::WrapI64ToI32(value) => Self::WrapI64ToI32(get(value)),
            Self::ExtendI32S(value) => Self::ExtendI32S(get(value)),
            Self::ExtendI32U(value) => Self::ExtendI32U(get(value)),
            Self::SelectI64 {
                condition,
                if_true,
                if_false,
            } => Self::SelectI64 {
                condition: get(condition),
                if_true: get(if_true),
                if_false: get(if_false),
            },
            Self::Load {
                address,
                kind,
                ref exit,
            } => Self::Load {
                address: get(address),
                kind,
                exit: exit.remap(map),
            },
            Self::ExactFp {
                op,
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                ref exit,
            } => Self::ExactFp {
                op,
                lhs: get(lhs),
                rhs: get(rhs),
                third: get(third),
                rm: get(rm),
                fcsr: get(fcsr),
                exit: exit.remap(map),
            },
            Self::Reservation { op, address } => Self::Reservation {
                op,
                address: get(address),
            },
            Self::ReloadFcsr(value) => Self::ReloadFcsr(get(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueData {
    pub ty: ValueType,
    pub op: Op,
    /// Guest PC responsible for this value, retained for diagnostics and for
    /// precise exits once ordered effects are introduced.
    pub guest_pc: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitKind {
    Dispatch,
    Unsupported,
    RegionLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub entry_pc: u64,
    pub end_pc: u64,
    pub values: Vec<ValueData>,
    /// Final SSA value for every architecturally dirty integer register.
    pub outputs: Vec<(u8, ValueId)>,
    /// Final raw IEEE bit pattern for dirty floating-point registers.
    pub f_outputs: Vec<(u8, ValueId)>,
    pub fcsr_output: Option<ValueId>,
    pub next_pc: ValueId,
    pub retired: u32,
    pub exit: ExitKind,
    /// Static guest-instruction mix retained for opt-in execution profiling.
    /// The emitter never reads it unless the embedding supplies profile cells.
    pub trace_mix: [u16; 5],
    /// Static memory operations whose decoded architectural base is x2.
    pub trace_stack_memory: u16,
    /// Whether this member commits a new architectural x2 value.
    pub writes_x2: bool,
    /// Ordered non-value effects. Loads live in the value graph and carry
    /// their own ordering point; stores are inserted at `position`.
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError(pub String);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

impl Region {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let ty = |id: ValueId| self.values.get(id.0).map(|v| v.ty);
        for (index, value) in self.values.iter().enumerate() {
            let id = ValueId(index);
            let check_prior = |operand: ValueId| -> Result<ValueType, ValidationError> {
                if operand.0 >= id.0 {
                    return Err(ValidationError(format!(
                        "value {index} refers to non-prior value {}",
                        operand.0
                    )));
                }
                ty(operand).ok_or_else(|| {
                    ValidationError(format!("value {index} has missing operand {}", operand.0))
                })
            };
            match value.op {
                Op::ConstI32(_) if value.ty == ValueType::I32 => {}
                Op::ConstI64(_) | Op::GuestPc(_) | Op::ReadX(_) | Op::ReadF(_)
                    if value.ty == ValueType::I64 => {}
                Op::ReadFcsr if value.ty == ValueType::I32 => {}
                Op::Binary { op, lhs, rhs } => {
                    if check_prior(lhs)? != op.input_type()
                        || check_prior(rhs)? != op.input_type()
                        || value.ty != op.result_type()
                    {
                        return Err(ValidationError(format!(
                            "value {index} has an invalid binary type"
                        )));
                    }
                }
                Op::Divide { op, lhs, rhs } => {
                    if check_prior(lhs)? != op.value_type()
                        || check_prior(rhs)? != op.value_type()
                        || value.ty != op.value_type()
                    {
                        return Err(ValidationError(format!(
                            "value {index} has an invalid divide type"
                        )));
                    }
                }
                Op::WrapI64ToI32(v)
                    if check_prior(v)? == ValueType::I64 && value.ty == ValueType::I32 => {}
                Op::ExtendI32S(v) | Op::ExtendI32U(v)
                    if check_prior(v)? == ValueType::I32 && value.ty == ValueType::I64 => {}
                Op::SelectI64 {
                    condition,
                    if_true,
                    if_false,
                } if check_prior(condition)? == ValueType::I32
                    && check_prior(if_true)? == ValueType::I64
                    && check_prior(if_false)? == ValueType::I64
                    && value.ty == ValueType::I64 => {}
                Op::Load {
                    address, ref exit, ..
                } if check_prior(address)? == ValueType::I64 && value.ty == ValueType::I64 => {
                    validate_side_exit(exit, &ty, index)?;
                }
                Op::ExactFp {
                    lhs,
                    rhs,
                    third,
                    rm,
                    fcsr,
                    ref exit,
                    ..
                } if check_prior(lhs)? == ValueType::I64
                    && check_prior(rhs)? == ValueType::I64
                    && check_prior(third)? == ValueType::I64
                    && check_prior(rm)? == ValueType::I32
                    && check_prior(fcsr)? == ValueType::I32
                    && value.ty == ValueType::I64 =>
                {
                    validate_side_exit(exit, &ty, index)?;
                }
                Op::Reservation { address, .. }
                    if check_prior(address)? == ValueType::I64 && value.ty == ValueType::I32 => {}
                Op::ReloadFcsr(helper)
                    if check_prior(helper)? == ValueType::I64 && value.ty == ValueType::I32 => {}
                _ => {
                    return Err(ValidationError(format!(
                        "value {index} has an invalid opcode/type pairing"
                    )));
                }
            }
        }
        if ty(self.next_pc) != Some(ValueType::I64) {
            return Err(ValidationError("next_pc is not i64".into()));
        }
        for &(reg, value) in &self.outputs {
            if reg == 0 || reg >= 32 || ty(value) != Some(ValueType::I64) {
                return Err(ValidationError(format!(
                    "invalid architectural output x{reg}"
                )));
            }
        }
        for &(reg, value) in &self.f_outputs {
            if reg >= 32 || ty(value) != Some(ValueType::I64) {
                return Err(ValidationError(format!(
                    "invalid floating-point output f{reg}"
                )));
            }
        }
        if self
            .fcsr_output
            .is_some_and(|value| ty(value) != Some(ValueType::I32))
        {
            return Err(ValidationError("invalid fcsr output".into()));
        }
        for effect in &self.effects {
            match effect {
                Effect::Store {
                    position,
                    address,
                    value,
                    condition,
                    exit,
                    ..
                } => {
                    if *position > self.values.len()
                        || address.0 >= *position
                        || value.0 >= *position
                        || ty(*address) != Some(ValueType::I64)
                        || ty(*value) != Some(ValueType::I64)
                        || condition.is_some_and(|condition| {
                            condition.0 >= *position || ty(condition) != Some(ValueType::I32)
                        })
                    {
                        return Err(ValidationError("invalid store effect".into()));
                    }
                    if let Some(condition) = condition {
                        let paired = self.values.get(condition.0).is_some_and(|data| {
                            matches!(
                                &data.op,
                                Op::Reservation {
                                    op: ReservationOp::StoreConditional,
                                    address: reservation_address,
                                } if *reservation_address == *address
                            )
                        });
                        if !paired {
                            return Err(ValidationError(
                                "conditional store lacks its SC reservation probe".into(),
                            ));
                        }
                    }
                    validate_side_exit(exit, &ty, *position)?;
                }
                Effect::Guard {
                    position,
                    condition,
                    exit,
                } => {
                    if *position > self.values.len()
                        || condition.0 >= *position
                        || ty(*condition) != Some(ValueType::I32)
                    {
                        return Err(ValidationError("invalid control guard".into()));
                    }
                    validate_side_exit(exit, &ty, *position)?;
                }
                Effect::GuardTarget {
                    position,
                    target,
                    exit,
                    ..
                } => {
                    if *position > self.values.len()
                        || target.0 >= *position
                        || ty(*target) != Some(ValueType::I64)
                    {
                        return Err(ValidationError("invalid indirect-target guard".into()));
                    }
                    validate_side_exit(exit, &ty, *position)?;
                }
                Effect::FpState { position, exit, .. } | Effect::VectorState { position, exit } => {
                    if *position > self.values.len() {
                        return Err(ValidationError("invalid FP-state effect".into()));
                    }
                    validate_side_exit(exit, &ty, *position)?;
                }
                Effect::Vector { position, exit, .. } => {
                    if *position > self.values.len() {
                        return Err(ValidationError("invalid vector effect".into()));
                    }
                    validate_side_exit(exit, &ty, *position)?;
                }
            }
        }
        Ok(())
    }

    /// Remove values not reachable from architectural outputs or the next PC.
    pub fn eliminate_dead_values(&mut self) {
        let mut live = vec![false; self.values.len()];
        let mut work = Vec::with_capacity(self.outputs.len() + 1);
        work.push(self.next_pc);
        work.extend(self.outputs.iter().map(|&(_, value)| value));
        work.extend(self.f_outputs.iter().map(|&(_, value)| value));
        work.extend(self.fcsr_output);
        for (index, value) in self.values.iter().enumerate() {
            if matches!(
                value.op,
                Op::Load { .. } | Op::ExactFp { .. } | Op::Reservation { .. }
            ) {
                work.push(ValueId(index));
            }
        }
        for effect in &self.effects {
            match effect {
                Effect::Store {
                    address,
                    value,
                    condition,
                    exit,
                    ..
                } => {
                    work.extend([*address, *value]);
                    work.extend(*condition);
                    exit.for_each_value(|value| work.push(value));
                }
                Effect::Guard {
                    condition, exit, ..
                } => {
                    work.push(*condition);
                    exit.for_each_value(|value| work.push(value));
                }
                Effect::GuardTarget { target, exit, .. } => {
                    work.push(*target);
                    exit.for_each_value(|value| work.push(value));
                }
                Effect::FpState { exit, .. } | Effect::VectorState { exit, .. } => {
                    exit.for_each_value(|value| work.push(value));
                }
                Effect::Vector { exit, .. } => {
                    exit.for_each_value(|value| work.push(value));
                }
            }
        }
        while let Some(id) = work.pop() {
            if live[id.0] {
                continue;
            }
            live[id.0] = true;
            self.values[id.0]
                .op
                .for_each_operand(|operand| work.push(operand));
        }

        let mut map = vec![None; self.values.len()];
        let mut values = Vec::with_capacity(live.iter().filter(|&&v| v).count());
        for (old, value) in self.values.iter().enumerate() {
            if live[old] {
                let new = ValueId(values.len());
                map[old] = Some(new);
                values.push(ValueData {
                    ty: value.ty,
                    op: value.op.remap(&map),
                    guest_pc: value.guest_pc,
                });
            }
        }
        let mut live_prefix = vec![0usize; live.len() + 1];
        for (index, is_live) in live.iter().copied().enumerate() {
            live_prefix[index + 1] = live_prefix[index] + usize::from(is_live);
        }
        for effect in &mut self.effects {
            match effect {
                Effect::Store {
                    position,
                    address,
                    value,
                    condition,
                    exit,
                    ..
                } => {
                    *position = live_prefix[*position];
                    *address = map[address.0].expect("store address is live");
                    *value = map[value.0].expect("store value is live");
                    *condition =
                        condition.map(|value| map[value.0].expect("store condition is live"));
                    *exit = exit.remap(&map);
                }
                Effect::Guard {
                    position,
                    condition,
                    exit,
                } => {
                    *position = live_prefix[*position];
                    *condition = map[condition.0].expect("guard condition is live");
                    *exit = exit.remap(&map);
                }
                Effect::GuardTarget {
                    position,
                    target,
                    exit,
                    ..
                } => {
                    *position = live_prefix[*position];
                    *target = map[target.0].expect("indirect guard target is live");
                    *exit = exit.remap(&map);
                }
                Effect::FpState { position, exit, .. } | Effect::VectorState { position, exit } => {
                    *position = live_prefix[*position];
                    *exit = exit.remap(&map);
                }
                Effect::Vector { position, exit, .. } => {
                    *position = live_prefix[*position];
                    *exit = exit.remap(&map);
                }
            }
        }
        self.values = values;
        self.next_pc = map[self.next_pc.0].expect("next pc is a live root");
        for (_, value) in &mut self.outputs {
            *value = map[value.0].expect("architectural output is a live root");
        }
        for (_, value) in &mut self.f_outputs {
            *value = map[value.0].expect("floating-point output is a live root");
        }
        if let Some(value) = &mut self.fcsr_output {
            *value = map[value.0].expect("fcsr output is a live root");
        }
    }

    pub fn use_counts(&self) -> Vec<u32> {
        let mut counts = vec![0; self.values.len()];
        for value in &self.values {
            value.op.for_each_operand(|operand| counts[operand.0] += 1);
            // Guarded division references both inputs on multiple structured
            // control paths. Force them into locals in the pure stackifier so
            // an expensive input expression is evaluated exactly once.
            if let Op::Divide { lhs, rhs, .. } = value.op {
                counts[lhs.0] += 1;
                counts[rhs.0] += 1;
            }
        }
        counts[self.next_pc.0] += 1;
        for &(_, value) in &self.outputs {
            counts[value.0] += 1;
        }
        for &(_, value) in &self.f_outputs {
            counts[value.0] += 1;
        }
        if let Some(value) = self.fcsr_output {
            counts[value.0] += 1;
        }
        for effect in &self.effects {
            match effect {
                Effect::Store {
                    address,
                    value,
                    condition,
                    exit,
                    ..
                } => {
                    counts[address.0] += 1;
                    counts[value.0] += 1;
                    if let Some(condition) = condition {
                        counts[condition.0] += 1;
                    }
                    exit.for_each_value(|output| counts[output.0] += 1);
                }
                Effect::Guard {
                    condition, exit, ..
                } => {
                    counts[condition.0] += 1;
                    exit.for_each_value(|output| counts[output.0] += 1);
                }
                Effect::GuardTarget { target, exit, .. } => {
                    counts[target.0] += 1;
                    exit.for_each_value(|output| counts[output.0] += 1);
                }
                Effect::FpState { exit, .. } | Effect::VectorState { exit, .. } => {
                    exit.for_each_value(|output| counts[output.0] += 1);
                }
                Effect::Vector { exit, .. } => {
                    exit.for_each_value(|output| counts[output.0] += 1);
                }
            }
        }
        counts
    }

    pub fn has_effects(&self) -> bool {
        !self.effects.is_empty()
            || self.values.iter().any(|v| {
                matches!(
                    v.op,
                    Op::Load { .. } | Op::ExactFp { .. } | Op::Reservation { .. }
                )
            })
    }

    pub fn has_fp_helper(&self) -> bool {
        self.values
            .iter()
            .any(|value| matches!(value.op, Op::ExactFp { .. }))
    }

    pub fn has_reservation_helper(&self) -> bool {
        self.values
            .iter()
            .any(|value| matches!(value.op, Op::Reservation { .. }))
    }

    pub fn has_vector_helper(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect, Effect::Vector { .. }))
    }
}

fn validate_side_exit(
    exit: &SideExit,
    ty: &impl Fn(ValueId) -> Option<ValueType>,
    available_before: usize,
) -> Result<(), ValidationError> {
    for &(reg, value) in &exit.outputs {
        if reg == 0 || reg >= 32 || value.0 >= available_before || ty(value) != Some(ValueType::I64)
        {
            return Err(ValidationError(format!("invalid side-exit output x{reg}")));
        }
    }
    for &(reg, value) in &exit.f_outputs {
        if reg >= 32 || value.0 >= available_before || ty(value) != Some(ValueType::I64) {
            return Err(ValidationError(format!(
                "invalid side-exit FP output f{reg}"
            )));
        }
    }
    if exit
        .fcsr_output
        .is_some_and(|value| value.0 >= available_before || ty(value) != Some(ValueType::I32))
    {
        return Err(ValidationError("invalid side-exit fcsr output".into()));
    }
    Ok(())
}

/// SSA construction helper with local constant folding.
pub(crate) struct Builder {
    entry_pc: u64,
    values: Vec<ValueData>,
    regs: [Option<ValueId>; 32],
    dirty: u32,
    fregs: [Option<ValueId>; 32],
    dirty_f: u32,
    fcsr: Option<ValueId>,
    dirty_fcsr: bool,
    effects: Vec<Effect>,
}

impl Builder {
    pub fn new(entry_pc: u64) -> Self {
        Self {
            entry_pc,
            values: Vec::new(),
            regs: [None; 32],
            dirty: 0,
            fregs: [None; 32],
            dirty_f: 0,
            fcsr: None,
            dirty_fcsr: false,
            effects: Vec::new(),
        }
    }

    fn push(&mut self, ty: ValueType, op: Op, guest_pc: u64) -> ValueId {
        let id = ValueId(self.values.len());
        self.values.push(ValueData { ty, op, guest_pc });
        id
    }

    pub fn ty(&self, value: ValueId) -> ValueType {
        self.values[value.0].ty
    }

    pub fn const_i32(&mut self, value: i32, pc: u64) -> ValueId {
        self.push(ValueType::I32, Op::ConstI32(value), pc)
    }

    pub fn const_i64(&mut self, value: i64, pc: u64) -> ValueId {
        self.push(ValueType::I64, Op::ConstI64(value), pc)
    }

    pub fn guest_pc(&mut self, value: u64, pc: u64) -> ValueId {
        self.push(ValueType::I64, Op::GuestPc(value), pc)
    }

    fn as_const_i32(&self, value: ValueId) -> Option<i32> {
        match self.values[value.0].op {
            Op::ConstI32(v) => Some(v),
            _ => None,
        }
    }

    fn as_const_i64(&self, value: ValueId) -> Option<i64> {
        match self.values[value.0].op {
            Op::ConstI64(v) => Some(v),
            _ => None,
        }
    }

    fn as_guest_pc(&self, value: ValueId) -> Option<u64> {
        match self.values[value.0].op {
            Op::GuestPc(v) => Some(v),
            _ => None,
        }
    }

    pub fn read_x(&mut self, reg: usize, pc: u64) -> ValueId {
        if reg == 0 {
            return self.const_i64(0, pc);
        }
        if let Some(value) = self.regs[reg] {
            return value;
        }
        let value = self.push(ValueType::I64, Op::ReadX(reg as u8), pc);
        self.regs[reg] = Some(value);
        value
    }

    pub fn write_x(&mut self, reg: usize, value: ValueId) {
        debug_assert_eq!(self.ty(value), ValueType::I64);
        if reg != 0 {
            self.regs[reg] = Some(value);
            self.dirty |= 1 << reg;
        }
    }

    pub fn read_f(&mut self, reg: usize, pc: u64) -> ValueId {
        if let Some(value) = self.fregs[reg] {
            return value;
        }
        let value = self.push(ValueType::I64, Op::ReadF(reg as u8), pc);
        self.fregs[reg] = Some(value);
        value
    }

    pub fn write_f(&mut self, reg: usize, value: ValueId) {
        debug_assert_eq!(self.ty(value), ValueType::I64);
        self.fregs[reg] = Some(value);
        self.dirty_f |= 1 << reg;
    }

    pub fn read_fcsr(&mut self, pc: u64) -> ValueId {
        if let Some(value) = self.fcsr {
            return value;
        }
        let value = self.push(ValueType::I32, Op::ReadFcsr, pc);
        self.fcsr = Some(value);
        value
    }

    pub fn write_fcsr(&mut self, value: ValueId) {
        debug_assert_eq!(self.ty(value), ValueType::I32);
        self.fcsr = Some(value);
        self.dirty_fcsr = true;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn exact_fp(
        &mut self,
        op: ExactFpOp,
        lhs: ValueId,
        rhs: ValueId,
        third: ValueId,
        rm: ValueId,
        guest_pc: u64,
        retired: u32,
    ) -> ValueId {
        debug_assert_eq!(self.ty(lhs), ValueType::I64);
        debug_assert_eq!(self.ty(rhs), ValueType::I64);
        debug_assert_eq!(self.ty(third), ValueType::I64);
        debug_assert_eq!(self.ty(rm), ValueType::I32);
        let fcsr = self.read_fcsr(guest_pc);
        let exit = self.side_exit(guest_pc, retired);
        let result = self.push(
            ValueType::I64,
            Op::ExactFp {
                op,
                lhs,
                rhs,
                third,
                rm,
                fcsr,
                exit,
            },
            guest_pc,
        );
        let updated_fcsr = self.push(ValueType::I32, Op::ReloadFcsr(result), guest_pc);
        self.write_fcsr(updated_fcsr);
        result
    }

    pub fn binary(&mut self, op: BinaryOp, lhs: ValueId, rhs: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(lhs), op.input_type());
        debug_assert_eq!(self.ty(rhs), op.input_type());

        if op.input_type() == ValueType::I64 {
            // Preserve the affine provenance of the compiler's common
            // AUIPC+ADDI address materialization.  A difference between two
            // PC-derived values is base-independent and may become an
            // ordinary constant; other operations remain explicit so a
            // position-independent emitter evaluates them after adding the
            // runtime page base.
            match op {
                BinaryOp::I64Add => {
                    if let (Some(base), Some(offset)) =
                        (self.as_guest_pc(lhs), self.as_const_i64(rhs))
                    {
                        return self.guest_pc(base.wrapping_add(offset as u64), pc);
                    }
                    if let (Some(offset), Some(base)) =
                        (self.as_const_i64(lhs), self.as_guest_pc(rhs))
                    {
                        return self.guest_pc(base.wrapping_add(offset as u64), pc);
                    }
                }
                BinaryOp::I64Sub => {
                    if let (Some(base), Some(offset)) =
                        (self.as_guest_pc(lhs), self.as_const_i64(rhs))
                    {
                        return self.guest_pc(base.wrapping_sub(offset as u64), pc);
                    }
                    if let (Some(lhs), Some(rhs)) = (self.as_guest_pc(lhs), self.as_guest_pc(rhs)) {
                        return self.const_i64(lhs.wrapping_sub(rhs) as i64, pc);
                    }
                }
                BinaryOp::I64And => {
                    // Architectural instruction addresses are two-byte
                    // aligned. JALR's mandatory low-bit clear therefore does
                    // not destroy PC-relative provenance for a statically
                    // materialized link/target.
                    if self
                        .as_guest_pc(lhs)
                        .is_some_and(|address| address & 1 == 0)
                        && self.as_const_i64(rhs) == Some(-2)
                    {
                        return lhs;
                    }
                    if self
                        .as_guest_pc(rhs)
                        .is_some_and(|address| address & 1 == 0)
                        && self.as_const_i64(lhs) == Some(-2)
                    {
                        return rhs;
                    }
                }
                _ => {}
            }
            if let (Some(a), Some(b)) = (self.as_const_i64(lhs), self.as_const_i64(rhs)) {
                let folded_i64 = match op {
                    BinaryOp::I64Add => Some(a.wrapping_add(b)),
                    BinaryOp::I64Sub => Some(a.wrapping_sub(b)),
                    BinaryOp::I64Mul => Some(a.wrapping_mul(b)),
                    BinaryOp::I64And => Some(a & b),
                    BinaryOp::I64Or => Some(a | b),
                    BinaryOp::I64Xor => Some(a ^ b),
                    BinaryOp::I64Shl => Some(a.wrapping_shl((b as u64 & 63) as u32)),
                    BinaryOp::I64ShrU => Some(((a as u64) >> (b as u64 & 63)) as i64),
                    BinaryOp::I64ShrS => Some(a >> (b as u64 & 63)),
                    _ => None,
                };
                if let Some(value) = folded_i64 {
                    return self.const_i64(value, pc);
                }
                let folded_i32 = match op {
                    BinaryOp::I64Eq => (a == b) as i32,
                    BinaryOp::I64Ne => (a != b) as i32,
                    BinaryOp::I64LtS => (a < b) as i32,
                    BinaryOp::I64LtU => ((a as u64) < (b as u64)) as i32,
                    BinaryOp::I64GeS => (a >= b) as i32,
                    BinaryOp::I64GeU => ((a as u64) >= (b as u64)) as i32,
                    _ => unreachable!(),
                };
                return self.const_i32(folded_i32, pc);
            }
            if self.as_const_i64(rhs) == Some(0) {
                match op {
                    BinaryOp::I64Add
                    | BinaryOp::I64Sub
                    | BinaryOp::I64Or
                    | BinaryOp::I64Xor
                    | BinaryOp::I64Shl
                    | BinaryOp::I64ShrU
                    | BinaryOp::I64ShrS => return lhs,
                    BinaryOp::I64Mul | BinaryOp::I64And => return rhs,
                    _ => {}
                }
            }
            if self.as_const_i64(rhs) == Some(1) && op == BinaryOp::I64Mul {
                return lhs;
            }
            if self.as_const_i64(rhs) == Some(-1) && op == BinaryOp::I64And {
                return lhs;
            }
        } else if let (Some(a), Some(b)) = (self.as_const_i32(lhs), self.as_const_i32(rhs)) {
            let value = match op {
                BinaryOp::I32Add => a.wrapping_add(b),
                BinaryOp::I32Sub => a.wrapping_sub(b),
                BinaryOp::I32Mul => a.wrapping_mul(b),
                BinaryOp::I32And => a & b,
                BinaryOp::I32Or => a | b,
                BinaryOp::I32Xor => a ^ b,
                BinaryOp::I32Shl => a.wrapping_shl(b as u32 & 31),
                BinaryOp::I32ShrU => ((a as u32) >> (b as u32 & 31)) as i32,
                BinaryOp::I32ShrS => a >> (b as u32 & 31),
                _ => unreachable!(),
            };
            return self.const_i32(value, pc);
        }

        self.push(op.result_type(), Op::Binary { op, lhs, rhs }, pc)
    }

    pub fn divide(&mut self, op: DivideOp, lhs: ValueId, rhs: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(lhs), op.value_type());
        debug_assert_eq!(self.ty(rhs), op.value_type());

        let folded = match op.value_type() {
            ValueType::I64 => {
                let (Some(lhs), Some(rhs)) = (self.as_const_i64(lhs), self.as_const_i64(rhs))
                else {
                    return self.push(ValueType::I64, Op::Divide { op, lhs, rhs }, pc);
                };
                let value = match op {
                    DivideOp::I64DivS if rhs == 0 => -1,
                    DivideOp::I64DivS if lhs == i64::MIN && rhs == -1 => lhs,
                    DivideOp::I64DivS => lhs / rhs,
                    DivideOp::I64DivU if rhs == 0 => -1,
                    DivideOp::I64DivU => ((lhs as u64) / (rhs as u64)) as i64,
                    DivideOp::I64RemS if rhs == 0 => lhs,
                    DivideOp::I64RemS if lhs == i64::MIN && rhs == -1 => 0,
                    DivideOp::I64RemS => lhs % rhs,
                    DivideOp::I64RemU if rhs == 0 => lhs,
                    DivideOp::I64RemU => ((lhs as u64) % (rhs as u64)) as i64,
                    _ => unreachable!(),
                };
                return self.const_i64(value, pc);
            }
            ValueType::I32 => {
                let (Some(lhs), Some(rhs)) = (self.as_const_i32(lhs), self.as_const_i32(rhs))
                else {
                    return self.push(ValueType::I32, Op::Divide { op, lhs, rhs }, pc);
                };
                match op {
                    DivideOp::I32DivS if rhs == 0 => -1,
                    DivideOp::I32DivS if lhs == i32::MIN && rhs == -1 => lhs,
                    DivideOp::I32DivS => lhs / rhs,
                    DivideOp::I32DivU if rhs == 0 => -1,
                    DivideOp::I32DivU => ((lhs as u32) / (rhs as u32)) as i32,
                    DivideOp::I32RemS if rhs == 0 => lhs,
                    DivideOp::I32RemS if lhs == i32::MIN && rhs == -1 => 0,
                    DivideOp::I32RemS => lhs % rhs,
                    DivideOp::I32RemU if rhs == 0 => lhs,
                    DivideOp::I32RemU => ((lhs as u32) % (rhs as u32)) as i32,
                    _ => unreachable!(),
                }
            }
        };
        self.const_i32(folded, pc)
    }

    /// Compute the high 64 bits of a 64x64 product using 32-bit limbs. Wasm
    /// has only low-half `i64.mul`; this expansion is portable across Wasm
    /// engines and avoids a JavaScript or interpreter helper transition.
    pub fn mul_high(&mut self, kind: MulHighKind, lhs: ValueId, rhs: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(lhs), ValueType::I64);
        debug_assert_eq!(self.ty(rhs), ValueType::I64);

        let mask = self.const_i64(0xffff_ffff, pc);
        let shift = self.const_i64(32, pc);
        let lhs_lo = self.binary(BinaryOp::I64And, lhs, mask, pc);
        let lhs_hi = self.binary(BinaryOp::I64ShrU, lhs, shift, pc);
        let rhs_lo = self.binary(BinaryOp::I64And, rhs, mask, pc);
        let rhs_hi = self.binary(BinaryOp::I64ShrU, rhs, shift, pc);

        let low_product = self.binary(BinaryOp::I64Mul, lhs_lo, rhs_lo, pc);
        let low_carry = self.binary(BinaryOp::I64ShrU, low_product, shift, pc);
        let left_cross = self.binary(BinaryOp::I64Mul, lhs_hi, rhs_lo, pc);
        let cross_with_carry = self.binary(BinaryOp::I64Add, left_cross, low_carry, pc);
        let cross_low = self.binary(BinaryOp::I64And, cross_with_carry, mask, pc);
        let cross_high = self.binary(BinaryOp::I64ShrU, cross_with_carry, shift, pc);
        let right_cross = self.binary(BinaryOp::I64Mul, lhs_lo, rhs_hi, pc);
        let right_with_cross = self.binary(BinaryOp::I64Add, cross_low, right_cross, pc);
        let right_carry = self.binary(BinaryOp::I64ShrU, right_with_cross, shift, pc);
        let high_product = self.binary(BinaryOp::I64Mul, lhs_hi, rhs_hi, pc);
        let high = self.binary(BinaryOp::I64Add, high_product, cross_high, pc);
        let mut high = self.binary(BinaryOp::I64Add, high, right_carry, pc);

        if matches!(
            kind,
            MulHighKind::SignedSigned | MulHighKind::SignedUnsigned
        ) {
            let zero = self.const_i64(0, pc);
            let negative = self.binary(BinaryOp::I64LtS, lhs, zero, pc);
            let correction = self.select_i64(negative, rhs, zero, pc);
            high = self.binary(BinaryOp::I64Sub, high, correction, pc);
        }
        if kind == MulHighKind::SignedSigned {
            let zero = self.const_i64(0, pc);
            let negative = self.binary(BinaryOp::I64LtS, rhs, zero, pc);
            let correction = self.select_i64(negative, lhs, zero, pc);
            high = self.binary(BinaryOp::I64Sub, high, correction, pc);
        }
        high
    }

    pub fn wrap_i32(&mut self, value: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(value), ValueType::I64);
        if let Some(value) = self.as_const_i64(value) {
            return self.const_i32(value as i32, pc);
        }
        self.push(ValueType::I32, Op::WrapI64ToI32(value), pc)
    }

    pub fn extend_i32_s(&mut self, value: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(value), ValueType::I32);
        if let Some(value) = self.as_const_i32(value) {
            return self.const_i64(value as i64, pc);
        }
        self.push(ValueType::I64, Op::ExtendI32S(value), pc)
    }

    pub fn extend_i32_u(&mut self, value: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(value), ValueType::I32);
        if let Some(value) = self.as_const_i32(value) {
            return self.const_i64(value as u32 as i64, pc);
        }
        self.push(ValueType::I64, Op::ExtendI32U(value), pc)
    }

    pub fn select_i64(
        &mut self,
        condition: ValueId,
        if_true: ValueId,
        if_false: ValueId,
        pc: u64,
    ) -> ValueId {
        debug_assert_eq!(self.ty(condition), ValueType::I32);
        debug_assert_eq!(self.ty(if_true), ValueType::I64);
        debug_assert_eq!(self.ty(if_false), ValueType::I64);
        if if_true == if_false {
            return if_true;
        }
        if let Some(condition) = self.as_const_i32(condition) {
            return if condition != 0 { if_true } else { if_false };
        }
        self.push(
            ValueType::I64,
            Op::SelectI64 {
                condition,
                if_true,
                if_false,
            },
            pc,
        )
    }

    fn side_exit(&self, guest_pc: u64, retired: u32) -> SideExit {
        let mut outputs = Vec::new();
        for reg in 1..32 {
            if self.dirty & (1 << reg) != 0 {
                outputs.push((
                    reg as u8,
                    self.regs[reg].expect("dirty register has a value"),
                ));
            }
        }
        let mut f_outputs = Vec::new();
        for reg in 0..32 {
            if self.dirty_f & (1 << reg) != 0 {
                f_outputs.push((
                    reg as u8,
                    self.fregs[reg].expect("dirty FP register has a value"),
                ));
            }
        }
        SideExit {
            guest_pc,
            retired,
            outputs,
            f_outputs,
            fcsr_output: self
                .dirty_fcsr
                .then(|| self.fcsr.expect("dirty fcsr has a value")),
        }
    }

    pub fn load(
        &mut self,
        address: ValueId,
        kind: LoadKind,
        guest_pc: u64,
        retired: u32,
    ) -> ValueId {
        debug_assert_eq!(self.ty(address), ValueType::I64);
        let exit = self.side_exit(guest_pc, retired);
        self.push(
            ValueType::I64,
            Op::Load {
                address,
                kind,
                exit,
            },
            guest_pc,
        )
    }

    pub fn store(
        &mut self,
        address: ValueId,
        value: ValueId,
        kind: StoreKind,
        guest_pc: u64,
        retired: u32,
    ) {
        debug_assert_eq!(self.ty(address), ValueType::I64);
        debug_assert_eq!(self.ty(value), ValueType::I64);
        self.effects.push(Effect::Store {
            position: self.values.len(),
            address,
            value,
            kind,
            condition: None,
            exit: self.side_exit(guest_pc, retired),
        });
    }

    pub fn store_conditional(
        &mut self,
        condition: ValueId,
        address: ValueId,
        value: ValueId,
        kind: StoreKind,
        guest_pc: u64,
        retired: u32,
    ) {
        debug_assert_eq!(self.ty(condition), ValueType::I32);
        debug_assert_eq!(self.ty(address), ValueType::I64);
        debug_assert_eq!(self.ty(value), ValueType::I64);
        self.effects.push(Effect::Store {
            position: self.values.len(),
            address,
            value,
            kind,
            condition: Some(condition),
            exit: self.side_exit(guest_pc, retired),
        });
    }

    pub fn reservation(&mut self, op: ReservationOp, address: ValueId, pc: u64) -> ValueId {
        debug_assert_eq!(self.ty(address), ValueType::I64);
        self.push(ValueType::I32, Op::Reservation { op, address }, pc)
    }

    pub fn fp_state(&mut self, dirty: bool, guest_pc: u64, retired: u32) {
        self.effects.push(Effect::FpState {
            position: self.values.len(),
            dirty,
            exit: self.side_exit(guest_pc, retired),
        });
    }

    pub fn vector_state(&mut self, guest_pc: u64, retired: u32) {
        self.effects.push(Effect::VectorState {
            position: self.values.len(),
            exit: self.side_exit(guest_pc, retired),
        });
    }

    /// Insert a complete RVV architectural effect and begin a new scalar SSA
    /// epoch. The helper may write any integer/FP register or fcsr, so no
    /// forwarded value is valid after this boundary.
    pub fn vector(&mut self, insn: u32, guest_pc: u64, fallthrough: u64, retired: u32) {
        self.effects.push(Effect::Vector {
            position: self.values.len(),
            insn,
            direct: VectorDirect::decode(insn),
            fallthrough,
            exit: self.side_exit(guest_pc, retired),
        });
        self.regs = [None; 32];
        self.dirty = 0;
        self.fregs = [None; 32];
        self.dirty_f = 0;
        self.fcsr = None;
        self.dirty_fcsr = false;
    }

    /// Add a post-instruction control side exit. `retired` includes the guest
    /// branch itself because the exit resumes at its selected successor.
    pub fn guard(&mut self, condition: ValueId, target_pc: u64, retired: u32) {
        debug_assert_eq!(self.ty(condition), ValueType::I32);
        self.effects.push(Effect::Guard {
            position: self.values.len(),
            condition,
            exit: self.side_exit(target_pc, retired),
        });
    }

    /// Add a monomorphic indirect-target guard. The ordinary path continues
    /// at `expected`; a mismatch resumes at the computed architectural target.
    pub fn guard_target(&mut self, target: ValueId, expected: u64, retired: u32) {
        debug_assert_eq!(self.ty(target), ValueType::I64);
        self.effects.push(Effect::GuardTarget {
            position: self.values.len(),
            target,
            expected,
            // `guest_pc` is a diagnostic fallback; the backend overwrites the
            // committed PC with `target` on the mismatch path.
            exit: self.side_exit(expected, retired),
        });
    }

    pub fn finish(self, end_pc: u64, next_pc: ValueId, retired: u32, exit: ExitKind) -> Region {
        let mut outputs = Vec::new();
        for reg in 1..32 {
            if self.dirty & (1 << reg) != 0 {
                outputs.push((
                    reg as u8,
                    self.regs[reg].expect("dirty register has a value"),
                ));
            }
        }
        let mut f_outputs = Vec::new();
        for reg in 0..32 {
            if self.dirty_f & (1 << reg) != 0 {
                f_outputs.push((
                    reg as u8,
                    self.fregs[reg].expect("dirty FP register has a value"),
                ));
            }
        }
        let mut region = Region {
            entry_pc: self.entry_pc,
            end_pc,
            values: self.values,
            outputs,
            f_outputs,
            fcsr_output: self
                .dirty_fcsr
                .then(|| self.fcsr.expect("dirty fcsr has a value")),
            next_pc,
            retired,
            exit,
            trace_mix: [0; 5],
            trace_stack_memory: 0,
            writes_x2: false,
            effects: self.effects,
        };
        region.eliminate_dead_values();
        debug_assert!(region.validate().is_ok());
        region
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector_encoding(funct6: u32, vm: bool, vs2: u8, source: u8, format: u32, vd: u8) -> u32 {
        0x57 | (u32::from(vd) << 7)
            | (format << 12)
            | (u32::from(source) << 15)
            | (u32::from(vs2) << 20)
            | (u32::from(vm) << 25)
            | (funct6 << 26)
    }

    #[test]
    fn direct_vector_decode_is_architectural_and_conservative() {
        let vadd = vector_encoding(0x00, true, 8, 12, 0, 16);
        assert_eq!(
            VectorDirect::decode(vadd),
            Some(VectorDirect::Lane {
                op: VectorLaneOp::Add,
                masked: false,
                destination: 16,
                source2: Some(8),
                operand: VectorOperand::Vector(12),
            })
        );
        assert_eq!(
            VectorDirect::decode(vadd & !(1 << 25)),
            Some(VectorDirect::Lane {
                op: VectorLaneOp::Add,
                masked: true,
                destination: 16,
                source2: Some(8),
                operand: VectorOperand::Vector(12),
            })
        );

        // Wasm shifts accept one scalar count, so the vector-count form stays
        // on the exact helper while immediate and scalar forms are candidates.
        assert!(VectorDirect::decode(vector_encoding(0x25, true, 8, 12, 0, 16)).is_none());
        assert!(matches!(
            VectorDirect::decode(vector_encoding(0x25, true, 8, 7, 3, 16)),
            Some(VectorDirect::Lane {
                op: VectorLaneOp::ShiftLeft,
                operand: VectorOperand::Immediate(7),
                ..
            })
        ));
        assert!(matches!(
            VectorDirect::decode(vector_encoding(0x25, true, 8, 31, 3, 16)),
            Some(VectorDirect::Lane {
                op: VectorLaneOp::ShiftLeft,
                operand: VectorOperand::Immediate(31),
                ..
            })
        ));
        assert!(matches!(
            VectorDirect::decode(vector_encoding(0x25, true, 8, 12, 2, 16)),
            Some(VectorDirect::Lane {
                op: VectorLaneOp::Multiply,
                operand: VectorOperand::Vector(12),
                ..
            })
        ));

        let vle32 = 0x07 | (16 << 7) | (6 << 12) | (10 << 15) | (1 << 25);
        assert_eq!(
            VectorDirect::decode(vle32),
            Some(VectorDirect::UnitStride {
                load: true,
                masked: false,
                width: 32,
                register: 16,
                base: 10,
            })
        );
        assert!(VectorDirect::decode(vle32 | (1 << 20)).is_none());
        assert_eq!(
            VectorDirect::decode(vle32 & !(1 << 25)),
            Some(VectorDirect::UnitStride {
                load: true,
                masked: true,
                width: 32,
                register: 16,
                base: 10,
            })
        );
        let vlse32 = vle32 | (2 << 26) | (13 << 20);
        assert_eq!(
            VectorDirect::decode(vlse32),
            Some(VectorDirect::Strided {
                load: true,
                masked: false,
                width: 32,
                register: 16,
                base: 10,
                stride: 13,
            })
        );

        assert_eq!(
            VectorDirect::decode(0x2285_6407), // vl2re32.v v8,(a0)
            Some(VectorDirect::WholeRegisterMemory {
                load: true,
                register: 8,
                base: 10,
                registers: 2,
            })
        );
        assert_eq!(
            VectorDirect::decode(0x2286_0427), // vs2r.v v8,(a2)
            Some(VectorDirect::WholeRegisterMemory {
                load: false,
                register: 8,
                base: 12,
                registers: 2,
            })
        );
        assert_eq!(
            VectorDirect::decode(0x9f01_b457), // vmv4r.v v8,v16
            Some(VectorDirect::WholeRegisterMove {
                destination: 8,
                source: 16,
                registers: 4,
            })
        );
        assert_eq!(
            VectorDirect::decode(0x4280_2557), // vmv.x.s a0,v8
            Some(VectorDirect::ScalarExtract {
                destination: 10,
                source: 8,
            })
        );
        assert_eq!(
            VectorDirect::decode(0x4206_e6d7), // vmv.s.x v13,a3
            Some(VectorDirect::ScalarInsert {
                destination: 13,
                source: 13,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0f, true, 8, 3, 3, 16)),
            Some(VectorDirect::SlideImmediate {
                up: false,
                destination: 16,
                source: 8,
                offset: 3,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0e, true, 8, 2, 3, 16)),
            Some(VectorDirect::SlideImmediate {
                up: true,
                destination: 16,
                source: 8,
                offset: 2,
            })
        );
        assert!(VectorDirect::decode(vector_encoding(0x0f, true, 8, 12, 4, 16)).is_none());
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0e, true, 8, 13, 6, 16)),
            Some(VectorDirect::SlideOne {
                up: true,
                destination: 16,
                source: 8,
                scalar: 13,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0f, true, 8, 13, 6, 16)),
            Some(VectorDirect::SlideOne {
                up: false,
                destination: 16,
                source: 8,
                scalar: 13,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0c, true, 8, 7, 3, 16)),
            Some(VectorDirect::GatherImmediate {
                destination: 16,
                source: 8,
                index: 7,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0c, true, 8, 12, 0, 16)),
            Some(VectorDirect::GatherVector {
                destination: 16,
                source: 8,
                indices: 12,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x14, true, 0, 0x11, 2, 16)),
            Some(VectorDirect::Index { destination: 16 })
        );
        assert!(VectorDirect::decode(vector_encoding(0x14, false, 0, 0x11, 2, 16)).is_none());
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x02, true, 8, 12, 2, 16)),
            Some(VectorDirect::Reduction {
                op: VectorReductionOp::Or,
                destination: 16,
                source: 8,
                seed: 12,
            })
        );
        assert!(VectorDirect::decode(vector_encoding(0x02, false, 8, 12, 2, 16)).is_none());
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x30, true, 9, 8, 2, 11)),
            Some(VectorDirect::WidenAddSub {
                signed: false,
                subtract: false,
                wide_left: false,
                destination: 11,
                source2: 9,
                operand: VectorOperand::Vector(8),
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x3c, true, 8, 29, 6, 11)),
            Some(VectorDirect::WidenMultiplyAccumulate {
                source2_signed: false,
                operand_signed: false,
                destination: 11,
                source2: 8,
                operand: VectorOperand::ScalarX(29),
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x09, true, 8, 12, 1, 16)),
            Some(VectorDirect::FloatSign {
                op: VectorFloatSignOp::Negate,
                destination: 16,
                source2: 8,
                source1: 12,
            })
        );
        assert!(VectorDirect::decode(vector_encoding(0x00, true, 8, 12, 1, 16)).is_none());
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x17, true, 0, 4, 5, 16)),
            Some(VectorDirect::FloatBroadcast {
                destination: 16,
                source: 4,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x10, true, 0, 4, 5, 16)),
            Some(VectorDirect::FloatScalarInsert {
                destination: 16,
                source: 4,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x10, true, 8, 0, 1, 5)),
            Some(VectorDirect::FloatScalarExtract {
                destination: 5,
                source: 8,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x0f, true, 8, 4, 5, 16)),
            Some(VectorDirect::FloatSlideOne {
                up: false,
                destination: 16,
                source: 8,
                scalar: 4,
            })
        );
        assert_eq!(
            VectorDirect::decode(vector_encoding(0x18, true, 8, 7, 3, 0)),
            Some(VectorDirect::Compare {
                op: VectorCompareOp::Equal,
                destination: 0,
                source2: 8,
                operand: VectorOperand::Immediate(7),
            })
        );
        assert!(VectorDirect::decode(vector_encoding(0x1a, true, 8, 7, 3, 0)).is_none());

        // Immediate configuration has an architecture-defined direct result;
        // FP arithmetic still never masquerades as integer SIMD.
        assert_eq!(
            VectorDirect::decode(0xcc08_7057),
            Some(VectorDirect::ConfigImmediate {
                destination: 0,
                vtype: 0xc0,
                vl: 16,
            })
        );
        assert_eq!(
            VectorDirect::decode(0x0da0_7557),
            Some(VectorDirect::ConfigImmediate {
                destination: 10,
                vtype: 0xda,
                vl: 8,
            })
        );
        // The retain-vl form is a candidate only for a full-VLMAX runtime
        // guard; its cold mismatched/partial cases remain with the helper.
        assert_eq!(
            VectorDirect::decode(0x0c80_7057),
            Some(VectorDirect::ConfigRetainFull {
                vtype: 0xc8,
                vlmax: 8,
            })
        );
        assert!(VectorDirect::decode(vector_encoding(0x00, true, 8, 12, 1, 16)).is_none());
    }

    #[test]
    fn state_forwarding_and_dce_remove_overwritten_writes() {
        let mut b = Builder::new(0x1000);
        let old = b.read_x(1, 0x1000);
        let one = b.const_i64(1, 0x1000);
        let first = b.binary(BinaryOp::I64Add, old, one, 0x1000);
        b.write_x(2, first);
        let second = b.const_i64(7, 0x1004);
        b.write_x(2, second);
        let next = b.const_i64(0x1008, 0x1004);
        let region = b.finish(0x1008, next, 2, ExitKind::Dispatch);
        assert_eq!(region.outputs, vec![(2, ValueId(0))]);
        assert!(matches!(region.values[0].op, Op::ConstI64(7)));
        assert!(region.validate().is_ok());
    }

    #[test]
    fn constants_fold_without_host_overflow() {
        let mut b = Builder::new(0);
        let max = b.const_i64(-1, 0);
        let one = b.const_i64(1, 0);
        let zero = b.binary(BinaryOp::I64Add, max, one, 0);
        assert_eq!(b.as_const_i64(zero), Some(0));
    }

    #[test]
    fn side_exit_keeps_the_pre_access_register_state_live() {
        let mut b = Builder::new(0x1000);
        let old = b.read_x(1, 0x1000);
        let one = b.const_i64(1, 0x1000);
        let updated = b.binary(BinaryOp::I64Add, old, one, 0x1000);
        b.write_x(1, updated);
        let address = b.read_x(2, 0x1004);
        // x3 is overwritten after the load, but the load itself must remain a
        // root because it can fault even if its result becomes dead.
        let loaded = b.load(address, LoadKind::I64, 0x1004, 1);
        b.write_x(3, loaded);
        let replacement = b.const_i64(9, 0x1008);
        b.write_x(3, replacement);
        let next = b.const_i64(0x100c, 0x1008);
        let region = b.finish(0x100c, next, 3, ExitKind::Dispatch);
        let exit = region
            .values
            .iter()
            .find_map(|value| match &value.op {
                Op::Load { exit, .. } => Some(exit),
                _ => None,
            })
            .expect("effectful load must survive DCE");
        assert_eq!(exit.retired, 1);
        assert!(exit.outputs.iter().any(|&(reg, _)| reg == 1));
        assert!(!exit.outputs.iter().any(|&(reg, _)| reg == 3));
        assert!(region.validate().is_ok());
    }

    #[test]
    fn high_multiply_limb_expansion_matches_wide_arithmetic() {
        let cases = [
            (0u64, 0u64),
            (1, u64::MAX),
            (u64::MAX, u64::MAX),
            (i64::MIN as u64, u64::MAX),
            (0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210),
        ];
        for (lhs_bits, rhs_bits) in cases {
            let mut b = Builder::new(0);
            let lhs = b.const_i64(lhs_bits as i64, 0);
            let rhs = b.const_i64(rhs_bits as i64, 0);
            let ss = b.mul_high(MulHighKind::SignedSigned, lhs, rhs, 0);
            let su = b.mul_high(MulHighKind::SignedUnsigned, lhs, rhs, 0);
            let uu = b.mul_high(MulHighKind::UnsignedUnsigned, lhs, rhs, 0);
            let expected_ss =
                (((lhs_bits as i64 as i128) * (rhs_bits as i64 as i128)) >> 64) as i64;
            let expected_su =
                (((lhs_bits as i64 as i128) * (rhs_bits as u128 as i128)) >> 64) as i64;
            let expected_uu = (((lhs_bits as u128) * (rhs_bits as u128)) >> 64) as u64 as i64;
            assert_eq!(b.as_const_i64(ss), Some(expected_ss));
            assert_eq!(b.as_const_i64(su), Some(expected_su));
            assert_eq!(b.as_const_i64(uu), Some(expected_uu));
        }
    }

    #[test]
    fn division_constant_folding_uses_riscv_edge_semantics() {
        let mut b = Builder::new(0);
        let min = b.const_i64(i64::MIN, 0);
        let minus_one = b.const_i64(-1, 0);
        let zero = b.const_i64(0, 0);
        let overflow = b.divide(DivideOp::I64DivS, min, minus_one, 0);
        let overflow_rem = b.divide(DivideOp::I64RemS, min, minus_one, 0);
        let zero_div = b.divide(DivideOp::I64DivU, min, zero, 0);
        let zero_rem = b.divide(DivideOp::I64RemU, min, zero, 0);
        assert_eq!(b.as_const_i64(overflow), Some(i64::MIN));
        assert_eq!(b.as_const_i64(overflow_rem), Some(0));
        assert_eq!(b.as_const_i64(zero_div), Some(-1));
        assert_eq!(b.as_const_i64(zero_rem), Some(i64::MIN));
    }

    #[test]
    fn conditional_store_requires_its_sc_reservation_probe() {
        let mut b = Builder::new(0x1000);
        let address = b.const_i64(0x2000, 0x1000);
        let value = b.const_i64(7, 0x1000);
        let condition = b.reservation(ReservationOp::StoreConditional, address, 0x1000);
        b.store_conditional(condition, address, value, StoreKind::I64, 0x1000, 0);
        let next = b.const_i64(0x1004, 0x1000);
        let mut region = b.finish(0x1004, next, 1, ExitKind::Dispatch);
        let condition = region
            .effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Store {
                    condition: Some(condition),
                    ..
                } => Some(*condition),
                _ => None,
            })
            .expect("conditional store");
        region.values[condition.0].op = Op::ConstI32(1);
        assert_eq!(
            region.validate().unwrap_err().0,
            "conditional store lacks its SC reservation probe"
        );
    }
}
