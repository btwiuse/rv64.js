/*
 * G001 proof-only FENCE.I-coherent decoded-interpreter model.
 *
 * This is not production emulator code. It isolates the exact representation
 * under consideration: the control fetches bytes and normalizes fields on
 * every execution, while the treatment uses one 64-entry direct-mapped cache
 * of at most 32 normalized scalar instructions. Both variants call the same
 * semantic dispatcher and update the same complete model state.
 */

#include <stddef.h>
#include <stdint.h>

#define EXPORT(name) __attribute__((export_name(name)))

#define CACHE_SLOTS 64u
#define BLOCK_LIMIT 32u
#define CODE_PAGES 64u
#define PAGE_SIZE 4096u
#define CODE_BASE 0x00040000ull
#define WRAP_BASE 0x00100000ull
#define STRADDLE_PC 0x00200ffeull
#define MUTATION_PC 0x00240000ull
#define DATA_BYTES 4096u
#define FLAT_LIMIT 320u
#define WRAP_BLOCKS 129u
#define OP32_COUNT 62u
#define C_BUCKET_COUNT 19u
#define C_PER_BUCKET 10u
#define EXPECTED_INSTRUCTIONS (OP32_COUNT + C_BUCKET_COUNT * C_PER_BUCKET)

enum {
  OP_LUI = 0,
  OP_AUIPC,
  OP_JAL,
  OP_JALR,
  OP_BEQ,
  OP_BNE,
  OP_BLT,
  OP_BGE,
  OP_BLTU,
  OP_BGEU,
  OP_LB,
  OP_LH,
  OP_LW,
  OP_LD,
  OP_LBU,
  OP_LHU,
  OP_LWU,
  OP_SB,
  OP_SH,
  OP_SW,
  OP_SD,
  OP_ADDI,
  OP_SLLI,
  OP_SLTI,
  OP_SLTIU,
  OP_XORI,
  OP_SRLI,
  OP_SRAI,
  OP_ORI,
  OP_ANDI,
  OP_ADDIW,
  OP_SLLIW,
  OP_SRLIW,
  OP_SRAIW,
  OP_ADD,
  OP_SUB,
  OP_SLL,
  OP_SLT,
  OP_SLTU,
  OP_XOR,
  OP_SRL,
  OP_SRA,
  OP_OR,
  OP_AND,
  OP_MUL,
  OP_MULH,
  OP_MULHSU,
  OP_MULHU,
  OP_DIV,
  OP_DIVU,
  OP_REM,
  OP_REMU,
  OP_ADDW,
  OP_SUBW,
  OP_SLLW,
  OP_SRLW,
  OP_SRAW,
  OP_MULW,
  OP_DIVW,
  OP_DIVUW,
  OP_REMW,
  OP_REMUW,

  OP_C_ADDI4SPN,
  OP_C_LW,
  OP_C_LD,
  OP_C_SW,
  OP_C_SD,
  OP_C_ADDI,
  OP_C_ADDIW,
  OP_C_LI,
  OP_C_ADDI16SP,
  OP_C_LUI,
  OP_C_SRLI,
  OP_C_SRAI,
  OP_C_ANDI,
  OP_C_SUB,
  OP_C_XOR,
  OP_C_OR,
  OP_C_AND,
  OP_C_SUBW,
  OP_C_ADDW,
  OP_C_J,
  OP_C_BEQZ,
  OP_C_BNEZ,
  OP_C_SLLI,
  OP_C_LWSP,
  OP_C_LDSP,
  OP_C_JR,
  OP_C_MV,
  OP_C_EBREAK,
  OP_C_JALR,
  OP_C_ADD,
  OP_C_SWSP,
  OP_C_SDSP,
  OP_COUNT
};

typedef struct {
  uint64_t pc;
  int64_t imm;
  uint32_t op;
  uint8_t rd;
  uint8_t rs1;
  uint8_t rs2;
  uint8_t len;
  uint32_t aux;
} Decoded;

typedef struct {
  uint64_t key_pc;
  uint64_t key_context;
  uint64_t key_map_generation;
  uint64_t key_ifetch_generation;
  uint32_t length;
  uint32_t valid;
  Decoded operations[BLOCK_LIMIT];
} CacheSlot;

typedef struct {
  uint8_t op;
  uint8_t balance;
} Token;

_Static_assert(sizeof(Decoded) == 32, "frozen normalized record must be 32 bytes");

static volatile uint8_t code_pages[CODE_PAGES][PAGE_SIZE];
static uint64_t block_pc[CODE_PAGES];
static uint8_t block_length[CODE_PAGES];
static uint32_t block_count_value;
static uint64_t flat_pc[FLAT_LIMIT];
static uint8_t flat_block[FLAT_LIMIT];
static uint8_t flat_position[FLAT_LIMIT];
static uint32_t flat_count_value;
static Token tokens[EXPECTED_INSTRUCTIONS];
static CacheSlot cache[CACHE_SLOTS];
static uint32_t wrap_words[WRAP_BLOCKS];
static uint32_t mutation_word;

static uint64_t registers_[32];
static uint8_t data_memory[DATA_BYTES];
static uint64_t architectural_pc;
static uint64_t pc_digest;
static uint64_t retired;

static uint64_t fetch_context;
static uint64_t map_generation;
static uint64_t ifetch_generation;
static uint64_t cache_hits_value;
static uint64_t cache_misses_value;
static uint32_t init_error;
static uint32_t error_index;
static uint32_t error_expected;
static uint32_t error_actual;

static int64_t sx(uint64_t value, unsigned bits) {
  uint64_t sign = 1ull << (bits - 1u);
  return (int64_t)((value ^ sign) - sign);
}

static uint64_t rotl64(uint64_t value, unsigned shift) {
  return (value << shift) | (value >> (64u - shift));
}

static uint32_t cache_index(uint64_t pc) {
  return (uint32_t)(((pc >> 1u) ^ (pc >> 12u)) & (CACHE_SLOTS - 1u));
}

static uint8_t register_for(uint32_t seed) {
  static const uint8_t allowed[] = {
      1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16,
      17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29};
  return allowed[seed % (sizeof(allowed) / sizeof(allowed[0]))];
}

static uint8_t prime_register_for(uint32_t seed) {
  return (uint8_t)(10u + seed % 6u);
}

static uint32_t enc_i(uint32_t opcode, uint32_t f3, uint8_t rd, uint8_t rs1,
                      int32_t imm) {
  return (((uint32_t)imm & 0xfffu) << 20u) | ((uint32_t)rs1 << 15u) |
         (f3 << 12u) | ((uint32_t)rd << 7u) | opcode;
}

static uint32_t enc_r(uint32_t opcode, uint32_t f3, uint32_t f7, uint8_t rd,
                      uint8_t rs1, uint8_t rs2) {
  return (f7 << 25u) | ((uint32_t)rs2 << 20u) | ((uint32_t)rs1 << 15u) |
         (f3 << 12u) | ((uint32_t)rd << 7u) | opcode;
}

static uint32_t enc_s(uint32_t f3, uint8_t rs1, uint8_t rs2, int32_t imm) {
  uint32_t value = (uint32_t)imm & 0xfffu;
  return ((value >> 5u) << 25u) | ((uint32_t)rs2 << 20u) |
         ((uint32_t)rs1 << 15u) | (f3 << 12u) | ((value & 0x1fu) << 7u) |
         0x23u;
}

static uint32_t enc_b(uint32_t f3, uint8_t rs1, uint8_t rs2, int32_t imm) {
  uint32_t value = (uint32_t)imm & 0x1fffu;
  return (((value >> 12u) & 1u) << 31u) | (((value >> 5u) & 0x3fu) << 25u) |
         ((uint32_t)rs2 << 20u) | ((uint32_t)rs1 << 15u) | (f3 << 12u) |
         (((value >> 1u) & 0xfu) << 8u) | (((value >> 11u) & 1u) << 7u) |
         0x63u;
}

static uint32_t enc_j(uint8_t rd, int32_t imm) {
  uint32_t value = (uint32_t)imm & 0x1fffffu;
  return (((value >> 20u) & 1u) << 31u) | (((value >> 1u) & 0x3ffu) << 21u) |
         (((value >> 11u) & 1u) << 20u) | (((value >> 12u) & 0xffu) << 12u) |
         ((uint32_t)rd << 7u) | 0x6fu;
}

static uint32_t encode_32(uint8_t op, uint32_t seed) {
  uint8_t rd = register_for(seed * 3u + 1u);
  uint8_t rs1 = register_for(seed * 5u + 2u);
  uint8_t rs2 = register_for(seed * 7u + 3u);
  int32_t imm = (seed & 1u) ? -(int32_t)(3u + seed % 61u)
                           : (int32_t)(5u + seed % 59u);
  uint32_t sh64 = (seed * 13u + 1u) & 63u;
  uint32_t sh32 = (seed * 11u + 1u) & 31u;

  if (op == OP_LUI || op == OP_AUIPC) {
    uint32_t opcode = op == OP_LUI ? 0x37u : 0x17u;
    return (((uint32_t)imm & 0xfffffu) << 12u) | ((uint32_t)rd << 7u) | opcode;
  }
  if (op == OP_JAL) return enc_j(rd, (seed & 1u) ? -64 : 60);
  if (op == OP_JALR) return enc_i(0x67u, 0u, rd, rs1, imm);
  if (op >= OP_BEQ && op <= OP_BGEU) {
    static const uint8_t f3[] = {0, 1, 4, 5, 6, 7};
    uint32_t branch = op - OP_BEQ;
    uint8_t left = 30;
    uint8_t right = (branch == 0u || branch == 1u) ? 30 : 31;
    return enc_b(f3[branch], left, right, (branch & 1u) ? -32 : 28);
  }
  if (op >= OP_LB && op <= OP_LWU) {
    return enc_i(0x03u, op - OP_LB, rd, rs1, imm);
  }
  if (op >= OP_SB && op <= OP_SD) {
    return enc_s(op - OP_SB, rs1, rs2, imm);
  }
  if (op >= OP_ADDI && op <= OP_ANDI) {
    static const uint8_t f3[] = {0, 1, 2, 3, 4, 5, 5, 6, 7};
    uint32_t index = op - OP_ADDI;
    int32_t encoded_imm = imm;
    if (op == OP_SLLI || op == OP_SRLI) encoded_imm = (int32_t)sh64;
    if (op == OP_SRAI) encoded_imm = (int32_t)(0x400u | sh64);
    return enc_i(0x13u, f3[index], rd, rs1, encoded_imm);
  }
  if (op >= OP_ADDIW && op <= OP_SRAIW) {
    static const uint8_t f3[] = {0, 1, 5, 5};
    int32_t encoded_imm = imm;
    if (op == OP_SLLIW || op == OP_SRLIW) encoded_imm = (int32_t)sh32;
    if (op == OP_SRAIW) encoded_imm = (int32_t)(0x400u | sh32);
    return enc_i(0x1bu, f3[op - OP_ADDIW], rd, rs1, encoded_imm);
  }
  if (op >= OP_ADD && op <= OP_REMU) {
    static const uint8_t base_f3[] = {0, 0, 1, 2, 3, 4, 5, 5, 6, 7};
    if (op <= OP_AND) {
      uint32_t index = op - OP_ADD;
      uint32_t f7 = (op == OP_SUB || op == OP_SRA) ? 0x20u : 0u;
      return enc_r(0x33u, base_f3[index], f7, rd, rs1, rs2);
    }
    return enc_r(0x33u, op - OP_MUL, 1u, rd, rs1, rs2);
  }
  if (op >= OP_ADDW && op <= OP_REMUW) {
    if (op <= OP_SRAW) {
      static const uint8_t f3[] = {0, 0, 1, 5, 5};
      uint32_t f7 = (op == OP_SUBW || op == OP_SRAW) ? 0x20u : 0u;
      return enc_r(0x3bu, f3[op - OP_ADDW], f7, rd, rs1, rs2);
    }
    static const uint8_t f3m[] = {0, 4, 5, 6, 7};
    return enc_r(0x3bu, f3m[op - OP_MULW], 1u, rd, rs1, rs2);
  }
  return 0u;
}

static uint16_t cbase(uint32_t quadrant, uint32_t f3) {
  return (uint16_t)((f3 << 13u) | quadrant);
}

static uint16_t encode_c(uint8_t op, uint32_t seed, uint32_t balance) {
  uint16_t c = 0;
  uint8_t rp = (uint8_t)(prime_register_for(seed) - 8u);
  uint8_t rp2 = (uint8_t)(prime_register_for(seed + 3u) - 8u);
  uint8_t rd = register_for(seed + 11u);
  uint8_t rs2 = register_for(seed + 17u);
  uint32_t shamt = (seed * 7u + 1u) & 63u;

  switch (op) {
    case OP_C_ADDI4SPN:
      c = cbase(0, 0) | (uint16_t)(rp << 2u) | (1u << 11u);
      break;
    case OP_C_LW:
      c = cbase(0, 2) | (uint16_t)(rp << 2u) | (uint16_t)(rp2 << 7u) |
          (1u << 10u) | (1u << 6u);
      break;
    case OP_C_LD:
      c = cbase(0, 3) | (uint16_t)(rp << 2u) | (uint16_t)(rp2 << 7u) |
          (1u << 10u) | (1u << 5u);
      break;
    case OP_C_SW:
      c = cbase(0, 6) | (uint16_t)(rp << 2u) | (uint16_t)(rp2 << 7u) |
          (1u << 10u) | (1u << 6u);
      break;
    case OP_C_SD:
      c = cbase(0, 7) | (uint16_t)(rp << 2u) | (uint16_t)(rp2 << 7u) |
          (1u << 10u) | (1u << 5u);
      break;
    case OP_C_ADDI:
      c = cbase(1, 0) | (uint16_t)(rd << 7u) | (uint16_t)(((seed % 31u) + 1u) << 2u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_ADDIW:
      c = cbase(1, 1) | (uint16_t)(rd << 7u) | (uint16_t)(((seed % 31u) + 1u) << 2u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_LI:
      c = cbase(1, 2) | (uint16_t)(rd << 7u) | (uint16_t)(((seed % 31u) + 1u) << 2u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_ADDI16SP:
      c = cbase(1, 3) | (2u << 7u) | (1u << 6u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_LUI:
      if (rd == 2u) rd = 3u;
      c = cbase(1, 3) | (uint16_t)(rd << 7u) | (1u << 2u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_SRLI:
    case OP_C_SRAI:
      c = cbase(1, 4) | (uint16_t)(rp << 7u) |
          (uint16_t)((op == OP_C_SRAI ? 1u : 0u) << 10u) |
          (uint16_t)((shamt & 31u) << 2u) | (uint16_t)(((shamt >> 5u) & 1u) << 12u);
      break;
    case OP_C_ANDI:
      c = cbase(1, 4) | (2u << 10u) | (uint16_t)(rp << 7u) |
          (uint16_t)(((seed % 31u) + 1u) << 2u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_SUB:
    case OP_C_XOR:
    case OP_C_OR:
    case OP_C_AND:
    case OP_C_SUBW:
    case OP_C_ADDW: {
      uint32_t selector = op - OP_C_SUB;
      uint32_t bit12 = selector >= 4u;
      uint32_t low = selector >= 4u ? selector - 4u : selector;
      c = cbase(1, 4) | (3u << 10u) | (uint16_t)(rp << 7u) |
          (uint16_t)(low << 5u) | (uint16_t)(rp2 << 2u) |
          (uint16_t)(bit12 << 12u);
      break;
    }
    case OP_C_J:
      c = cbase(1, 5) | (1u << 3u);
      if (seed & 1u) c |= 1u << 12u;
      break;
    case OP_C_BEQZ:
    case OP_C_BNEZ: {
      uint8_t branch_rp = (uint8_t)((balance & 1u) ? 1u : 0u);
      c = cbase(1, op == OP_C_BEQZ ? 6u : 7u) |
          (uint16_t)(branch_rp << 7u) | (1u << 3u);
      break;
    }
    case OP_C_SLLI:
      c = cbase(2, 0) | (uint16_t)(rd << 7u) |
          (uint16_t)((shamt & 31u) << 2u) | (uint16_t)(((shamt >> 5u) & 1u) << 12u);
      break;
    case OP_C_LWSP:
      c = cbase(2, 2) | (uint16_t)(rd << 7u) | (1u << 4u) | (1u << 2u);
      break;
    case OP_C_LDSP:
      c = cbase(2, 3) | (uint16_t)(rd << 7u) | (1u << 5u) | (1u << 2u);
      break;
    case OP_C_JR:
      c = cbase(2, 4) | (uint16_t)(rd << 7u);
      break;
    case OP_C_MV:
      c = cbase(2, 4) | (uint16_t)(rd << 7u) | (uint16_t)(rs2 << 2u);
      break;
    case OP_C_EBREAK:
      c = cbase(2, 4) | (1u << 12u);
      break;
    case OP_C_JALR:
      c = cbase(2, 4) | (1u << 12u) | (uint16_t)(rd << 7u);
      break;
    case OP_C_ADD:
      c = cbase(2, 4) | (1u << 12u) | (uint16_t)(rd << 7u) |
          (uint16_t)(rs2 << 2u);
      break;
    case OP_C_SWSP:
      c = cbase(2, 6) | (1u << 9u) | (uint16_t)(rs2 << 2u);
      break;
    case OP_C_SDSP:
      c = cbase(2, 7) | (1u << 10u) | (uint16_t)(rs2 << 2u);
      break;
    default:
      break;
  }
  return c;
}

static uint32_t c_bucket(uint32_t op) {
  if (op < OP_C_ADDI4SPN || op >= OP_COUNT) return 0xffu;
  if (op <= OP_C_SD) return op - OP_C_ADDI4SPN;
  if (op == OP_C_ADDI) return 5;
  if (op == OP_C_ADDIW) return 6;
  if (op == OP_C_LI) return 7;
  if (op == OP_C_ADDI16SP || op == OP_C_LUI) return 8;
  if (op >= OP_C_SRLI && op <= OP_C_ADDW) return 9;
  if (op == OP_C_J) return 10;
  if (op == OP_C_BEQZ) return 11;
  if (op == OP_C_BNEZ) return 12;
  if (op == OP_C_SLLI) return 13;
  if (op == OP_C_LWSP) return 14;
  if (op == OP_C_LDSP) return 15;
  if (op >= OP_C_JR && op <= OP_C_ADD) return 16;
  if (op == OP_C_SWSP) return 17;
  return 18;
}

static uint32_t is_control(uint32_t op) {
  return (op >= OP_JAL && op <= OP_BGEU) || op == OP_C_J ||
         op == OP_C_BEQZ || op == OP_C_BNEZ || op == OP_C_JR ||
         op == OP_C_EBREAK || op == OP_C_JALR;
}

static uint8_t code_byte(uint64_t pc) {
  uint64_t relative = pc - CODE_BASE;
  uint32_t page = (uint32_t)(relative >> 12u);
  uint32_t offset = (uint32_t)(relative & (PAGE_SIZE - 1u));
  if (page >= CODE_PAGES) {
    init_error = 90;
    return 0;
  }
  return code_pages[page][offset];
}

static uint16_t fetch16(uint64_t pc) {
  return (uint16_t)code_byte(pc) | ((uint16_t)code_byte(pc + 1u) << 8u);
}

static void decoded_base(Decoded *d, uint64_t pc, uint32_t op, uint8_t rd,
                         uint8_t rs1, uint8_t rs2, int64_t imm, uint8_t len) {
  d->pc = pc;
  d->imm = imm;
  d->op = op;
  d->rd = rd;
  d->rs1 = rs1;
  d->rs2 = rs2;
  d->len = len;
  d->aux = 0;
}

static uint32_t decode32_word(uint32_t word, uint64_t pc, Decoded *d) {
  uint32_t opcode = word & 0x7fu;
  uint8_t rd = (uint8_t)((word >> 7u) & 31u);
  uint32_t f3 = (word >> 12u) & 7u;
  uint8_t rs1 = (uint8_t)((word >> 15u) & 31u);
  uint8_t rs2 = (uint8_t)((word >> 20u) & 31u);
  uint32_t f7 = word >> 25u;
  int64_t imm_i = (int32_t)word >> 20u;
  int64_t imm_s = sx(((word >> 7u) & 0x1fu) | ((word >> 20u) & 0xfe0u), 12);
  int64_t imm_b = sx(((word >> 7u) & 0x1eu) | ((word >> 20u) & 0x7e0u) |
                         ((word << 4u) & 0x800u) | ((word >> 19u) & 0x1000u),
                     13);
  int64_t imm_j = sx(((word >> 20u) & 0x7feu) | ((word >> 9u) & 0x800u) |
                         (word & 0xff000u) | ((word >> 11u) & 0x100000u),
                     21);
  uint32_t op = OP_COUNT;
  int64_t imm = 0;

  switch (opcode) {
    case 0x37:
      op = OP_LUI;
      imm = (int32_t)(word & 0xfffff000u);
      break;
    case 0x17:
      op = OP_AUIPC;
      imm = (int32_t)(word & 0xfffff000u);
      break;
    case 0x6f:
      op = OP_JAL;
      imm = imm_j;
      break;
    case 0x67:
      if (f3 != 0u) return 0;
      op = OP_JALR;
      imm = imm_i;
      break;
    case 0x63: {
      static const uint8_t map[] = {OP_BEQ, OP_BNE, 0xff, 0xff,
                                    OP_BLT, OP_BGE, OP_BLTU, OP_BGEU};
      op = map[f3];
      imm = imm_b;
      break;
    }
    case 0x03:
      if (f3 > 6u) return 0;
      op = OP_LB + f3;
      imm = imm_i;
      break;
    case 0x23:
      if (f3 > 3u) return 0;
      op = OP_SB + f3;
      imm = imm_s;
      break;
    case 0x13:
      if (f3 == 0u) op = OP_ADDI;
      else if (f3 == 1u && f7 <= 1u) op = OP_SLLI;
      else if (f3 == 2u) op = OP_SLTI;
      else if (f3 == 3u) op = OP_SLTIU;
      else if (f3 == 4u) op = OP_XORI;
      else if (f3 == 5u && (word >> 26u) == 0u) op = OP_SRLI;
      else if (f3 == 5u && (word >> 26u) == 0x10u) op = OP_SRAI;
      else if (f3 == 6u) op = OP_ORI;
      else if (f3 == 7u) op = OP_ANDI;
      imm = imm_i;
      break;
    case 0x1b:
      if (f3 == 0u) op = OP_ADDIW;
      else if (f3 == 1u && f7 == 0u) op = OP_SLLIW;
      else if (f3 == 5u && f7 == 0u) op = OP_SRLIW;
      else if (f3 == 5u && f7 == 0x20u) op = OP_SRAIW;
      imm = imm_i;
      break;
    case 0x33:
      if (f7 == 1u) op = OP_MUL + f3;
      else if (f7 == 0u) {
        static const uint8_t map[] = {OP_ADD, OP_SLL, OP_SLT, OP_SLTU,
                                      OP_XOR, OP_SRL, OP_OR, OP_AND};
        op = map[f3];
      } else if (f7 == 0x20u && f3 == 0u) op = OP_SUB;
      else if (f7 == 0x20u && f3 == 5u) op = OP_SRA;
      break;
    case 0x3b:
      if (f7 == 1u) {
        static const uint8_t map[] = {OP_MULW, 0xff, 0xff, 0xff,
                                      OP_DIVW, OP_DIVUW, OP_REMW, OP_REMUW};
        op = map[f3];
      } else if (f7 == 0u && f3 == 0u) op = OP_ADDW;
      else if (f7 == 0x20u && f3 == 0u) op = OP_SUBW;
      else if (f7 == 0u && f3 == 1u) op = OP_SLLW;
      else if (f7 == 0u && f3 == 5u) op = OP_SRLW;
      else if (f7 == 0x20u && f3 == 5u) op = OP_SRAW;
      break;
    default:
      break;
  }
  if (op >= OP_COUNT) return 0;
  decoded_base(d, pc, op, rd, rs1, rs2, imm, 4);
  return 1;
}

static uint32_t decode_c_word(uint16_t c, uint64_t pc, Decoded *d) {
  uint32_t q = c & 3u;
  uint32_t f3 = c >> 13u;
  uint8_t rd = (uint8_t)((c >> 7u) & 31u);
  uint8_t rs2 = (uint8_t)((c >> 2u) & 31u);
  uint8_t rdp = (uint8_t)(8u + ((c >> 7u) & 7u));
  uint8_t rs2p = (uint8_t)(8u + ((c >> 2u) & 7u));
  uint32_t op = OP_COUNT;
  int64_t imm = 0;
  uint8_t rs1 = rd;

  if (q == 0u) {
    rdp = (uint8_t)(8u + ((c >> 2u) & 7u));
    rs1 = (uint8_t)(8u + ((c >> 7u) & 7u));
    rs2 = rdp;
    if (f3 == 0u) {
      op = OP_C_ADDI4SPN;
      rs1 = 2;
      imm = (((c >> 11u) & 3u) << 4u) | (((c >> 7u) & 15u) << 6u) |
            (((c >> 6u) & 1u) << 2u) | (((c >> 5u) & 1u) << 3u);
    } else if (f3 == 2u || f3 == 6u) {
      op = f3 == 2u ? OP_C_LW : OP_C_SW;
      imm = (((c >> 10u) & 7u) << 3u) | (((c >> 6u) & 1u) << 2u) |
            (((c >> 5u) & 1u) << 6u);
    } else if (f3 == 3u || f3 == 7u) {
      op = f3 == 3u ? OP_C_LD : OP_C_SD;
      imm = (((c >> 10u) & 7u) << 3u) | (((c >> 5u) & 3u) << 6u);
    }
    rd = rdp;
  } else if (q == 1u) {
    imm = sx((((c >> 12u) & 1u) << 5u) | ((c >> 2u) & 31u), 6);
    if (f3 == 0u) op = OP_C_ADDI;
    else if (f3 == 1u) op = OP_C_ADDIW;
    else if (f3 == 2u) op = OP_C_LI;
    else if (f3 == 3u && rd == 2u) {
      op = OP_C_ADDI16SP;
      rs1 = 2;
      imm = sx((((c >> 12u) & 1u) << 9u) | (((c >> 6u) & 1u) << 4u) |
                   (((c >> 5u) & 1u) << 6u) | (((c >> 3u) & 3u) << 7u) |
                   (((c >> 2u) & 1u) << 5u),
               10);
    } else if (f3 == 3u) {
      op = OP_C_LUI;
      imm <<= 12u;
    } else if (f3 == 4u) {
      rd = rdp;
      rs1 = rdp;
      uint32_t sub = (c >> 10u) & 3u;
      if (sub == 0u || sub == 1u) {
        op = sub == 0u ? OP_C_SRLI : OP_C_SRAI;
        imm = (((c >> 12u) & 1u) << 5u) | ((c >> 2u) & 31u);
      } else if (sub == 2u) op = OP_C_ANDI;
      else {
        uint32_t selector = (((c >> 12u) & 1u) << 2u) | ((c >> 5u) & 3u);
        static const uint8_t map[] = {OP_C_SUB, OP_C_XOR, OP_C_OR, OP_C_AND,
                                      OP_C_SUBW, OP_C_ADDW, 0xff, 0xff};
        op = map[selector];
        rs2 = rs2p;
      }
    } else if (f3 == 5u) {
      op = OP_C_J;
      rd = 0;
      rs1 = 0;
      rs2 = 0;
      imm = sx((((c >> 12u) & 1u) << 11u) | (((c >> 11u) & 1u) << 4u) |
                   (((c >> 9u) & 3u) << 8u) | (((c >> 8u) & 1u) << 10u) |
                   (((c >> 7u) & 1u) << 6u) | (((c >> 6u) & 1u) << 7u) |
                   (((c >> 3u) & 7u) << 1u) | (((c >> 2u) & 1u) << 5u),
               12);
    } else if (f3 == 6u || f3 == 7u) {
      op = f3 == 6u ? OP_C_BEQZ : OP_C_BNEZ;
      rs1 = rdp;
      rd = 0;
      rs2 = 0;
      imm = sx((((c >> 12u) & 1u) << 8u) | (((c >> 10u) & 3u) << 3u) |
                   (((c >> 5u) & 3u) << 6u) | (((c >> 3u) & 3u) << 1u) |
                   (((c >> 2u) & 1u) << 5u),
               9);
    }
  } else if (q == 2u) {
    if (f3 == 0u) {
      op = OP_C_SLLI;
      imm = (((c >> 12u) & 1u) << 5u) | ((c >> 2u) & 31u);
    } else if (f3 == 2u) {
      op = OP_C_LWSP;
      rs1 = 2;
      imm = (((c >> 12u) & 1u) << 5u) | (((c >> 4u) & 7u) << 2u) |
            (((c >> 2u) & 3u) << 6u);
    } else if (f3 == 3u) {
      op = OP_C_LDSP;
      rs1 = 2;
      imm = (((c >> 12u) & 1u) << 5u) | (((c >> 5u) & 3u) << 3u) |
            (((c >> 2u) & 7u) << 6u);
    } else if (f3 == 4u) {
      uint32_t bit12 = (c >> 12u) & 1u;
      if (!bit12 && rd != 0u && rs2 == 0u) {
        op = OP_C_JR;
        rs1 = rd;
      } else if (!bit12 && rd != 0u && rs2 != 0u) {
        op = OP_C_MV;
        rs1 = 0;
      } else if (bit12 && rd == 0u && rs2 == 0u) {
        op = OP_C_EBREAK;
      } else if (bit12 && rd != 0u && rs2 == 0u) {
        op = OP_C_JALR;
        rs1 = rd;
        rd = 1;
      } else if (bit12 && rd != 0u && rs2 != 0u) {
        op = OP_C_ADD;
        rs1 = rd;
      }
    } else if (f3 == 6u) {
      op = OP_C_SWSP;
      rs1 = 2;
      imm = (((c >> 9u) & 15u) << 2u) | (((c >> 7u) & 3u) << 6u);
    } else if (f3 == 7u) {
      op = OP_C_SDSP;
      rs1 = 2;
      imm = (((c >> 10u) & 7u) << 3u) | (((c >> 7u) & 7u) << 6u);
    }
  }
  if (op >= OP_COUNT) return 0;
  decoded_base(d, pc, op, rd, rs1, rs2, imm, 2);
  return 1;
}

static uint32_t decode_at(uint64_t pc, Decoded *d) {
  uint16_t lo = fetch16(pc);
  if ((lo & 3u) != 3u) return decode_c_word(lo, pc, d);
  uint32_t word = lo | ((uint32_t)fetch16(pc + 2u) << 16u);
  return decode32_word(word, pc, d);
}

static void write_code(uint64_t pc, uint32_t word, uint8_t length) {
  uint64_t relative = pc - CODE_BASE;
  uint32_t page = (uint32_t)(relative >> 12u);
  uint32_t offset = (uint32_t)(relative & (PAGE_SIZE - 1u));
  if (page >= CODE_PAGES || offset + length > PAGE_SIZE) {
    init_error = 91;
    return;
  }
  for (uint32_t byte = 0; byte < length; byte++) {
    code_pages[page][offset + byte] = (uint8_t)(word >> (byte * 8u));
  }
}

static uint32_t next_random(uint32_t *state) {
  uint32_t value = *state;
  value ^= value << 13u;
  value ^= value >> 17u;
  value ^= value << 5u;
  *state = value;
  return value;
}

static void append_token(uint32_t *count, uint8_t op, uint8_t balance) {
  if (*count >= EXPECTED_INSTRUCTIONS) {
    init_error = 1;
    return;
  }
  tokens[*count].op = op;
  tokens[*count].balance = balance;
  (*count)++;
}

static void make_tokens(void) {
  uint32_t count = 0;
  for (uint32_t op = 0; op < OP32_COUNT; op++) append_token(&count, (uint8_t)op, 0);

  static const uint8_t variants[C_BUCKET_COUNT][9] = {
      {OP_C_ADDI4SPN},
      {OP_C_LW},
      {OP_C_LD},
      {OP_C_SW},
      {OP_C_SD},
      {OP_C_ADDI},
      {OP_C_ADDIW},
      {OP_C_LI},
      {OP_C_ADDI16SP, OP_C_LUI},
      {OP_C_SRLI, OP_C_SRAI, OP_C_ANDI, OP_C_SUB, OP_C_XOR,
       OP_C_OR, OP_C_AND, OP_C_SUBW, OP_C_ADDW},
      {OP_C_J},
      {OP_C_BEQZ},
      {OP_C_BNEZ},
      {OP_C_SLLI},
      {OP_C_LWSP},
      {OP_C_LDSP},
      {OP_C_JR, OP_C_MV, OP_C_EBREAK, OP_C_JALR, OP_C_ADD},
      {OP_C_SWSP},
      {OP_C_SDSP},
  };
  static const uint8_t variant_counts[C_BUCKET_COUNT] = {
      1, 1, 1, 1, 1, 1, 1, 1, 2, 9, 1, 1, 1, 1, 1, 1, 5, 1, 1};
  for (uint32_t bucket = 0; bucket < C_BUCKET_COUNT; bucket++) {
    for (uint32_t rep = 0; rep < C_PER_BUCKET; rep++) {
      append_token(&count, variants[bucket][rep % variant_counts[bucket]], (uint8_t)rep);
    }
  }
  if (count != EXPECTED_INSTRUCTIONS) init_error = 2;

  uint32_t random = 0x6a09e667u;
  for (uint32_t i = count - 1u; i > 0u; i--) {
    uint32_t j = next_random(&random) % (i + 1u);
    Token temporary = tokens[i];
    tokens[i] = tokens[j];
    tokens[j] = temporary;
  }
  if (!is_control(tokens[count - 1u].op)) {
    for (uint32_t i = 0; i + 1u < count; i++) {
      if (is_control(tokens[i].op)) {
        Token temporary = tokens[i];
        tokens[i] = tokens[count - 1u];
        tokens[count - 1u] = temporary;
        break;
      }
    }
  }
}

static uint64_t pc_for_block(uint32_t block) {
  uint64_t offset = (uint64_t)(block & 15u) << 8u;
  return CODE_BASE + (uint64_t)block * PAGE_SIZE + offset;
}

static void build_blocks(void) {
  uint32_t block = 0;
  uint32_t position = 0;
  uint32_t flat = 0;
  uint64_t pc = pc_for_block(0);
  block_pc[0] = pc;

  for (uint32_t index = 0; index < EXPECTED_INSTRUCTIONS; index++) {
    uint8_t op = tokens[index].op;
    uint8_t length = op < OP32_COUNT ? 4u : 2u;
    uint32_t word = op < OP32_COUNT
                        ? encode_32(op, index)
                        : encode_c(op, index, tokens[index].balance);
    write_code(pc, word, length);
    flat_pc[flat] = pc;
    flat_block[flat] = (uint8_t)block;
    flat_position[flat] = (uint8_t)position;

    Decoded decoded;
    if (!decode_at(pc, &decoded) || decoded.op != op || decoded.len != length) {
      init_error = 10u + index;
      error_index = index;
      error_expected = op;
      error_actual = decoded.op;
      return;
    }
    flat++;
    position++;
    pc += length;

    if (is_control(op) || position == BLOCK_LIMIT) {
      block_length[block] = (uint8_t)position;
      block++;
      position = 0;
      if (index + 1u < EXPECTED_INSTRUCTIONS) {
        if (block >= CODE_PAGES) {
          init_error = 3;
          return;
        }
        pc = pc_for_block(block);
        block_pc[block] = pc;
      }
    }
  }
  if (position != 0u || flat != EXPECTED_INSTRUCTIONS) {
    init_error = 4;
    return;
  }
  block_count_value = block;
  flat_count_value = flat;
}

static void write_register(uint8_t index, uint64_t value) {
  if (index != 0u) registers_[index] = value;
}

static uint32_t memory_address(uint64_t base, int64_t imm, uint32_t width) {
  uint64_t value = base + (uint64_t)imm;
  return (uint32_t)(value % (DATA_BYTES - width + 1u));
}

static uint64_t load_data(uint32_t address, uint32_t width) {
  uint64_t value = 0;
  for (uint32_t byte = 0; byte < width; byte++) {
    value |= (uint64_t)data_memory[address + byte] << (byte * 8u);
  }
  return value;
}

static void store_data(uint32_t address, uint32_t width, uint64_t value) {
  for (uint32_t byte = 0; byte < width; byte++) {
    data_memory[address + byte] = (uint8_t)(value >> (byte * 8u));
  }
}

static int64_t signed_div64(int64_t a, int64_t b) {
  if (b == 0) return -1;
  if (a == (int64_t)0x8000000000000000ull && b == -1) return a;
  return a / b;
}

static int64_t signed_rem64(int64_t a, int64_t b) {
  if (b == 0) return a;
  if (a == (int64_t)0x8000000000000000ull && b == -1) return 0;
  return a % b;
}

static int32_t signed_div32(int32_t a, int32_t b) {
  if (b == 0) return -1;
  if (a == (int32_t)0x80000000u && b == -1) return a;
  return a / b;
}

static int32_t signed_rem32(int32_t a, int32_t b) {
  if (b == 0) return a;
  if (a == (int32_t)0x80000000u && b == -1) return 0;
  return a % b;
}

static uint64_t multiply_high_unsigned(uint64_t a, uint64_t b) {
  uint64_t a0 = (uint32_t)a;
  uint64_t a1 = a >> 32u;
  uint64_t b0 = (uint32_t)b;
  uint64_t b1 = b >> 32u;
  uint64_t w0 = a0 * b0;
  uint64_t t = a1 * b0 + (w0 >> 32u);
  uint64_t w1 = (uint32_t)t;
  uint64_t w2 = t >> 32u;
  w1 += a0 * b1;
  return a1 * b1 + w2 + (w1 >> 32u);
}

static uint64_t multiply_high_signed(uint64_t a, uint64_t b) {
  uint64_t high = multiply_high_unsigned(a, b);
  if ((int64_t)a < 0) high -= b;
  if ((int64_t)b < 0) high -= a;
  return high;
}

static uint64_t multiply_high_signed_unsigned(uint64_t a, uint64_t b) {
  uint64_t high = multiply_high_unsigned(a, b);
  if ((int64_t)a < 0) high -= b;
  return high;
}

static void execute_decoded(const Decoded *d) {
  uint64_t a = registers_[d->rs1];
  uint64_t b = registers_[d->rs2];
  uint64_t result = 0;
  uint64_t next_pc = d->pc + d->len;
  uint32_t write = 0;
  uint32_t width = 0;
  uint32_t address = 0;
  uint32_t taken = 0;

  switch (d->op) {
    case OP_LUI: result = (uint64_t)d->imm; write = 1; break;
    case OP_AUIPC: result = d->pc + (uint64_t)d->imm; write = 1; break;
    case OP_JAL:
      result = d->pc + 4u; write = 1; next_pc = d->pc + (uint64_t)d->imm; break;
    case OP_JALR:
      result = d->pc + 4u; write = 1; next_pc = (a + (uint64_t)d->imm) & ~1ull; break;
    case OP_BEQ: taken = a == b; goto branch_done;
    case OP_BNE: taken = a != b; goto branch_done;
    case OP_BLT: taken = (int64_t)a < (int64_t)b; goto branch_done;
    case OP_BGE: taken = (int64_t)a >= (int64_t)b; goto branch_done;
    case OP_BLTU: taken = a < b; goto branch_done;
    case OP_BGEU:
      taken = a >= b;
    branch_done:
      if (taken) next_pc = d->pc + (uint64_t)d->imm;
      break;

    case OP_LB: width = 1; result = (uint64_t)(int64_t)(int8_t)load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LH: width = 2; result = (uint64_t)(int64_t)(int16_t)load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LW: width = 4; result = (uint64_t)(int64_t)(int32_t)load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LD: width = 8; result = load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LBU: width = 1; result = load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LHU: width = 2; result = load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_LWU: width = 4; result = load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_SB: width = 1; goto store_done;
    case OP_SH: width = 2; goto store_done;
    case OP_SW: width = 4; goto store_done;
    case OP_SD:
      width = 8;
    store_done:
      address = memory_address(a, d->imm, width);
      store_data(address, width, b);
      break;

    case OP_ADDI: result = a + (uint64_t)d->imm; write = 1; break;
    case OP_SLLI: result = a << ((uint64_t)d->imm & 63u); write = 1; break;
    case OP_SLTI: result = (int64_t)a < d->imm; write = 1; break;
    case OP_SLTIU: result = a < (uint64_t)d->imm; write = 1; break;
    case OP_XORI: result = a ^ (uint64_t)d->imm; write = 1; break;
    case OP_SRLI: result = a >> ((uint64_t)d->imm & 63u); write = 1; break;
    case OP_SRAI: result = (uint64_t)((int64_t)a >> ((uint64_t)d->imm & 63u)); write = 1; break;
    case OP_ORI: result = a | (uint64_t)d->imm; write = 1; break;
    case OP_ANDI: result = a & (uint64_t)d->imm; write = 1; break;
    case OP_ADDIW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a + (uint32_t)d->imm); write = 1; break;
    case OP_SLLIW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a << ((uint32_t)d->imm & 31u)); write = 1; break;
    case OP_SRLIW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a >> ((uint32_t)d->imm & 31u)); write = 1; break;
    case OP_SRAIW: result = (uint64_t)(int64_t)((int32_t)a >> ((uint32_t)d->imm & 31u)); write = 1; break;

    case OP_ADD: result = a + b; write = 1; break;
    case OP_SUB: result = a - b; write = 1; break;
    case OP_SLL: result = a << (b & 63u); write = 1; break;
    case OP_SLT: result = (int64_t)a < (int64_t)b; write = 1; break;
    case OP_SLTU: result = a < b; write = 1; break;
    case OP_XOR: result = a ^ b; write = 1; break;
    case OP_SRL: result = a >> (b & 63u); write = 1; break;
    case OP_SRA: result = (uint64_t)((int64_t)a >> (b & 63u)); write = 1; break;
    case OP_OR: result = a | b; write = 1; break;
    case OP_AND: result = a & b; write = 1; break;
    case OP_MUL: result = a * b; write = 1; break;
    case OP_MULH: result = multiply_high_signed(a, b); write = 1; break;
    case OP_MULHSU: result = multiply_high_signed_unsigned(a, b); write = 1; break;
    case OP_MULHU: result = multiply_high_unsigned(a, b); write = 1; break;
    case OP_DIV: result = (uint64_t)signed_div64((int64_t)a, (int64_t)b); write = 1; break;
    case OP_DIVU: result = b == 0 ? UINT64_MAX : a / b; write = 1; break;
    case OP_REM: result = (uint64_t)signed_rem64((int64_t)a, (int64_t)b); write = 1; break;
    case OP_REMU: result = b == 0 ? a : a % b; write = 1; break;
    case OP_ADDW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a + (uint32_t)b); write = 1; break;
    case OP_SUBW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a - (uint32_t)b); write = 1; break;
    case OP_SLLW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a << (b & 31u)); write = 1; break;
    case OP_SRLW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a >> (b & 31u)); write = 1; break;
    case OP_SRAW: result = (uint64_t)(int64_t)((int32_t)a >> (b & 31u)); write = 1; break;
    case OP_MULW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a * (uint32_t)b); write = 1; break;
    case OP_DIVW: result = (uint64_t)(int64_t)signed_div32((int32_t)a, (int32_t)b); write = 1; break;
    case OP_DIVUW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)b == 0 ? UINT32_MAX : (uint32_t)a / (uint32_t)b); write = 1; break;
    case OP_REMW: result = (uint64_t)(int64_t)signed_rem32((int32_t)a, (int32_t)b); write = 1; break;
    case OP_REMUW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)b == 0 ? (uint32_t)a : (uint32_t)a % (uint32_t)b); write = 1; break;

    case OP_C_ADDI4SPN: result = registers_[2] + (uint64_t)d->imm; write = 1; break;
    case OP_C_LW:
    case OP_C_LWSP: width = 4; result = (uint64_t)(int64_t)(int32_t)load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_C_LD:
    case OP_C_LDSP: width = 8; result = load_data(memory_address(a, d->imm, width), width); write = 1; break;
    case OP_C_SW:
    case OP_C_SWSP: width = 4; address = memory_address(a, d->imm, width); store_data(address, width, b); break;
    case OP_C_SD:
    case OP_C_SDSP: width = 8; address = memory_address(a, d->imm, width); store_data(address, width, b); break;
    case OP_C_ADDI: result = a + (uint64_t)d->imm; write = 1; break;
    case OP_C_ADDIW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a + (uint32_t)d->imm); write = 1; break;
    case OP_C_LI: result = (uint64_t)d->imm; write = 1; break;
    case OP_C_ADDI16SP: result = registers_[2] + (uint64_t)d->imm; write = 1; break;
    case OP_C_LUI: result = (uint64_t)d->imm; write = 1; break;
    case OP_C_SRLI: result = a >> ((uint64_t)d->imm & 63u); write = 1; break;
    case OP_C_SRAI: result = (uint64_t)((int64_t)a >> ((uint64_t)d->imm & 63u)); write = 1; break;
    case OP_C_ANDI: result = a & (uint64_t)d->imm; write = 1; break;
    case OP_C_SUB: result = a - b; write = 1; break;
    case OP_C_XOR: result = a ^ b; write = 1; break;
    case OP_C_OR: result = a | b; write = 1; break;
    case OP_C_AND: result = a & b; write = 1; break;
    case OP_C_SUBW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a - (uint32_t)b); write = 1; break;
    case OP_C_ADDW: result = (uint64_t)(int64_t)(int32_t)((uint32_t)a + (uint32_t)b); write = 1; break;
    case OP_C_J: next_pc = d->pc + (uint64_t)d->imm; break;
    case OP_C_BEQZ: if (a == 0) next_pc = d->pc + (uint64_t)d->imm; break;
    case OP_C_BNEZ: if (a != 0) next_pc = d->pc + (uint64_t)d->imm; break;
    case OP_C_SLLI: result = a << ((uint64_t)d->imm & 63u); write = 1; break;
    case OP_C_JR: next_pc = a & ~1ull; break;
    case OP_C_MV: result = b; write = 1; break;
    case OP_C_EBREAK: next_pc ^= 0x8000000000000000ull; break;
    case OP_C_JALR: result = d->pc + 2u; write = 1; next_pc = a & ~1ull; break;
    case OP_C_ADD: result = a + b; write = 1; break;
    default: break;
  }

  if (write) write_register(d->rd, result);
  registers_[0] = 0;
  architectural_pc = next_pc;
  pc_digest = rotl64(pc_digest, 7) ^ next_pc ^ ((uint64_t)d->op << 48u);
  retired++;
}

static CacheSlot *lookup_main_block(uint32_t block) {
  uint64_t pc = block_pc[block];
  CacheSlot *slot = &cache[cache_index(pc)];
  if (slot->valid && slot->key_pc == pc && slot->key_context == fetch_context &&
      slot->key_map_generation == map_generation &&
      slot->key_ifetch_generation == ifetch_generation) {
    cache_hits_value++;
    return slot;
  }

  cache_misses_value++;
  slot->valid = 0;
  uint64_t cursor = pc;
  uint32_t length = 0;
  while (length < BLOCK_LIMIT && (cursor & (PAGE_SIZE - 1u)) <= PAGE_SIZE - 2u) {
    Decoded decoded;
    if (!decode_at(cursor, &decoded)) break;
    if (decoded.len == 4u && (cursor & (PAGE_SIZE - 1u)) == PAGE_SIZE - 2u) break;
    slot->operations[length] = decoded;
    length++;
    cursor += decoded.len;
    if (is_control(decoded.op)) break;
  }
  slot->key_pc = pc;
  slot->key_context = fetch_context;
  slot->key_map_generation = map_generation;
  slot->key_ifetch_generation = ifetch_generation;
  slot->length = length;
  slot->valid = 1;
  if (length != block_length[block]) init_error = 70u + block;
  return slot;
}

static void execute_control_block(uint32_t block) {
  uint64_t pc = block_pc[block];
  for (uint32_t position = 0; position < BLOCK_LIMIT; position++) {
    Decoded decoded;
    if (!decode_at(pc, &decoded)) {
      init_error = 200u + block;
      return;
    }
    execute_decoded(&decoded);
    pc += decoded.len;
    if (is_control(decoded.op)) break;
  }
}

static void execute_treatment_block(uint32_t block) {
  CacheSlot *slot = lookup_main_block(block);
  for (uint32_t position = 0; position < slot->length; position++) {
    execute_decoded(&slot->operations[position]);
  }
}

static uint64_t state_checksum(void) {
  uint64_t value = architectural_pc ^ pc_digest ^ retired;
  for (uint32_t index = 0; index < 32u; index++) value = rotl64(value, 5) ^ registers_[index];
  for (uint32_t index = 0; index < DATA_BYTES; index += 8u) {
    value = rotl64(value, 3) ^ load_data(index, 8);
  }
  return value;
}

EXPORT("init_model") uint32_t init_model(void) {
  init_error = 0;
  error_index = 0;
  error_expected = 0;
  error_actual = 0;
  for (uint32_t page = 0; page < CODE_PAGES; page++) {
    for (uint32_t byte = 0; byte < PAGE_SIZE; byte++) code_pages[page][byte] = 0;
    block_pc[page] = 0;
    block_length[page] = 0;
  }
  make_tokens();
  if (!init_error) build_blocks();
  for (uint32_t index = 0; index < WRAP_BLOCKS; index++) {
    wrap_words[index] = encode_32((uint8_t)(OP_ADDI + index % 9u), index + 1000u);
  }
  mutation_word = encode_32(OP_ADDI, 4000u);
  fetch_context = 3;
  map_generation = 1;
  ifetch_generation = 1;
  return init_error;
}

EXPORT("reset_state") void reset_state(void) {
  uint64_t seed = 0x243f6a8885a308d3ull;
  registers_[0] = 0;
  for (uint32_t index = 1; index < 32u; index++) {
    seed += 0x9e3779b97f4a7c15ull;
    uint64_t value = seed;
    value = (value ^ (value >> 30u)) * 0xbf58476d1ce4e5b9ull;
    value = (value ^ (value >> 27u)) * 0x94d049bb133111ebull;
    registers_[index] = value ^ (value >> 31u) ^ index;
  }
  registers_[8] = 0;
  registers_[9] = 1;
  registers_[30] = 0;
  registers_[31] = 1;
  for (uint32_t index = 0; index < DATA_BYTES; index++) {
    data_memory[index] = (uint8_t)((index * 73u + 19u) ^ (index >> 3u));
  }
  architectural_pc = 0;
  pc_digest = 0x13198a2e03707344ull;
  retired = 0;
}

EXPORT("reset_cache") void reset_cache(void) {
  for (uint32_t index = 0; index < CACHE_SLOTS; index++) cache[index].valid = 0;
  cache_hits_value = 0;
  cache_misses_value = 0;
}

EXPORT("run_control") uint64_t run_control(uint32_t rounds) {
  for (uint32_t round = 0; round < rounds; round++) {
    for (uint32_t block = 0; block < block_count_value; block++) execute_control_block(block);
  }
  return state_checksum();
}

EXPORT("run_treatment") uint64_t run_treatment(uint32_t rounds) {
  for (uint32_t round = 0; round < rounds; round++) {
    for (uint32_t block = 0; block < block_count_value; block++) execute_treatment_block(block);
  }
  return state_checksum();
}

EXPORT("run_single_control") uint64_t run_single_control(uint32_t index) {
  if (index >= flat_count_value) return 0;
  Decoded decoded;
  if (!decode_at(flat_pc[index], &decoded)) return 0;
  execute_decoded(&decoded);
  return state_checksum();
}

EXPORT("run_single_treatment") uint64_t run_single_treatment(uint32_t index) {
  if (index >= flat_count_value) return 0;
  CacheSlot *slot = lookup_main_block(flat_block[index]);
  uint32_t position = flat_position[index];
  if (position >= slot->length) return 0;
  execute_decoded(&slot->operations[position]);
  return state_checksum();
}

static void execute_wrap_control(uint32_t index) {
  Decoded decoded;
  decode32_word(wrap_words[index], WRAP_BASE + (uint64_t)index * PAGE_SIZE, &decoded);
  execute_decoded(&decoded);
}

static void execute_wrap_treatment(uint32_t index) {
  uint64_t pc = WRAP_BASE + (uint64_t)index * PAGE_SIZE;
  CacheSlot *slot = &cache[cache_index(pc)];
  if (slot->valid && slot->key_pc == pc && slot->key_context == fetch_context &&
      slot->key_map_generation == map_generation &&
      slot->key_ifetch_generation == ifetch_generation) {
    cache_hits_value++;
  } else {
    cache_misses_value++;
    decode32_word(wrap_words[index], pc, &slot->operations[0]);
    slot->key_pc = pc;
    slot->key_context = fetch_context;
    slot->key_map_generation = map_generation;
    slot->key_ifetch_generation = ifetch_generation;
    slot->length = 1;
    slot->valid = 1;
  }
  execute_decoded(&slot->operations[0]);
}

EXPORT("run_wrap_control") uint64_t run_wrap_control(uint32_t passes) {
  for (uint32_t pass = 0; pass < passes; pass++) {
    for (uint32_t index = 0; index < WRAP_BLOCKS; index++) execute_wrap_control(index);
  }
  return state_checksum();
}

EXPORT("run_wrap_treatment") uint64_t run_wrap_treatment(uint32_t passes) {
  for (uint32_t pass = 0; pass < passes; pass++) {
    for (uint32_t index = 0; index < WRAP_BLOCKS; index++) execute_wrap_treatment(index);
  }
  return state_checksum();
}

EXPORT("run_straddle_control") uint64_t run_straddle_control(void) {
  Decoded decoded;
  uint32_t word = encode_32(OP_XORI, 5000u);
  decode32_word(word, STRADDLE_PC, &decoded);
  execute_decoded(&decoded);
  return state_checksum();
}

EXPORT("run_straddle_treatment") uint64_t run_straddle_treatment(void) {
  /* Frozen G001 falls back to the authoritative split fetch at page + 0xffe. */
  return run_straddle_control();
}

static CacheSlot *lookup_mutation(void) {
  CacheSlot *slot = &cache[cache_index(MUTATION_PC)];
  if (slot->valid && slot->key_pc == MUTATION_PC &&
      slot->key_context == fetch_context && slot->key_map_generation == map_generation &&
      slot->key_ifetch_generation == ifetch_generation) {
    cache_hits_value++;
    return slot;
  }
  cache_misses_value++;
  decode32_word(mutation_word, MUTATION_PC, &slot->operations[0]);
  slot->key_pc = MUTATION_PC;
  slot->key_context = fetch_context;
  slot->key_map_generation = map_generation;
  slot->key_ifetch_generation = ifetch_generation;
  slot->length = 1;
  slot->valid = 1;
  return slot;
}

EXPORT("set_mutation_version") void set_mutation_version(uint32_t version) {
  mutation_word = encode_32(version ? OP_XORI : OP_ADDI, 4000u);
}

EXPORT("run_mutation_control") uint64_t run_mutation_control(void) {
  Decoded decoded;
  decode32_word(mutation_word, MUTATION_PC, &decoded);
  execute_decoded(&decoded);
  return state_checksum();
}

EXPORT("run_mutation_treatment") uint64_t run_mutation_treatment(void) {
  CacheSlot *slot = lookup_mutation();
  execute_decoded(&slot->operations[0]);
  return state_checksum();
}

EXPORT("fence_i") void fence_i(void) { ifetch_generation++; }
EXPORT("set_fetch_context") void set_fetch_context(uint64_t value) { fetch_context = value; }
EXPORT("bump_map_generation") void bump_map_generation(void) { map_generation++; }

EXPORT("state_word_count") uint32_t state_word_count(void) { return 35u + DATA_BYTES / 8u; }
EXPORT("state_word") uint64_t state_word(uint32_t index) {
  if (index < 32u) return registers_[index];
  if (index == 32u) return architectural_pc;
  if (index == 33u) return pc_digest;
  if (index == 34u) return retired;
  index -= 35u;
  if (index < DATA_BYTES / 8u) return load_data(index * 8u, 8u);
  return 0;
}

EXPORT("flat_count") uint32_t flat_count(void) { return flat_count_value; }
EXPORT("block_count") uint32_t block_count(void) { return block_count_value; }
EXPORT("block_pc_at") uint64_t block_pc_at(uint32_t index) { return index < block_count_value ? block_pc[index] : 0; }
EXPORT("block_length_at") uint32_t block_length_at(uint32_t index) { return index < block_count_value ? block_length[index] : 0; }
EXPORT("flat_field") uint64_t flat_field(uint32_t index, uint32_t field) {
  if (index >= flat_count_value) return 0;
  Decoded decoded;
  if (!decode_at(flat_pc[index], &decoded)) return 0;
  if (field == 0u) return decoded.op;
  if (field == 1u) return decoded.rd;
  if (field == 2u) return decoded.rs1;
  if (field == 3u) return decoded.rs2;
  if (field == 4u) return (uint64_t)decoded.imm;
  if (field == 5u) return decoded.len;
  if (field == 6u) return decoded.pc;
  if (field == 7u) return flat_block[index];
  if (field == 8u) return flat_position[index];
  return 0;
}
EXPORT("flat_raw_byte") uint32_t flat_raw_byte(uint32_t index, uint32_t byte) {
  if (index >= flat_count_value || byte >= 4u) return 0;
  return code_byte(flat_pc[index] + byte);
}
EXPORT("compressed_bucket_for_op") uint32_t compressed_bucket_for_op(uint32_t op) { return c_bucket(op); }
EXPORT("control_for_op") uint32_t control_for_op(uint32_t op) { return is_control(op); }
EXPORT("cache_index_for_pc") uint32_t cache_index_for_pc(uint64_t pc) { return cache_index(pc); }
EXPORT("cache_hits") uint64_t cache_hits(void) { return cache_hits_value; }
EXPORT("cache_misses") uint64_t cache_misses(void) { return cache_misses_value; }
EXPORT("ifetch_generation_value") uint64_t ifetch_generation_value(void) { return ifetch_generation; }
EXPORT("model_error") uint32_t model_error(void) { return init_error; }
EXPORT("model_error_index") uint32_t model_error_index(void) { return error_index; }
EXPORT("model_error_expected") uint32_t model_error_expected(void) { return error_expected; }
EXPORT("model_error_actual") uint32_t model_error_actual(void) { return error_actual; }
EXPORT("wrap_block_count") uint32_t wrap_block_count(void) { return WRAP_BLOCKS; }
EXPORT("cache_slots") uint32_t cache_slots(void) { return CACHE_SLOTS; }
EXPORT("block_limit") uint32_t block_limit(void) { return BLOCK_LIMIT; }
EXPORT("normalized_record_bytes") uint32_t normalized_record_bytes(void) { return sizeof(Decoded); }
