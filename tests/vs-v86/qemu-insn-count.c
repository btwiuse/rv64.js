// QEMU user-mode instruction counter used to separate emulator throughput
// from cross-ISA workload volume in the scorecard's native binaries.
//
// Each guest instruction increments exactly one broad semantic class. Data
// reads and writes are counted independently through QEMU's memory-access
// instrumentation, so they include implicit x86 accesses as well as explicit
// RISC-V loads/stores. The plugin is diagnostic only: its instrumentation
// overhead makes its wall time unsuitable as a benchmark result.
#include <ctype.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include <glib.h>
#include <qemu-plugin.h>

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

enum instruction_class {
    CLASS_LOAD_8,
    CLASS_LOAD_16,
    CLASS_LOAD_32,
    CLASS_LOAD_64,
    CLASS_STORE_8,
    CLASS_STORE_16,
    CLASS_STORE_32,
    CLASS_STORE_64,
    CLASS_COPY_IMMEDIATE,
    CLASS_ADD_SUB,
    CLASS_LOGICAL,
    CLASS_SHIFT,
    CLASS_COMPARE,
    CLASS_CONDITIONAL_BRANCH,
    CLASS_DIRECT_CONTROL,
    CLASS_INDIRECT_CONTROL,
    CLASS_MULTIPLY,
    CLASS_DIVIDE_REMAINDER,
    CLASS_ATOMIC,
    CLASS_FP_MEMORY,
    CLASS_FP_ARITHMETIC,
    CLASS_FENCE,
    CLASS_SYSTEM,
    CLASS_X86_STRING,
    CLASS_OTHER,
    CLASS_COUNT,
};

// Dynamic writes to the architectural stack pointer.  This is deliberately
// semantic rather than mnemonic-only: QEMU may print compressed instructions
// using their canonical aliases, and an addi whose source is not sp is a copy,
// not an affine update of an existing stack proof.
enum sp_write_class {
    SP_WRITE_AFFINE_IMMEDIATE,
    SP_WRITE_AFFINE_REGISTER,
    SP_WRITE_COPY_OR_CONSTANT,
    SP_WRITE_LOAD,
    SP_WRITE_OTHER,
    SP_WRITE_CLASS_COUNT,
};

static const char *const sp_write_class_names[SP_WRITE_CLASS_COUNT] = {
    [SP_WRITE_AFFINE_IMMEDIATE] = "AFFINE_IMMEDIATE",
    [SP_WRITE_AFFINE_REGISTER] = "AFFINE_REGISTER",
    [SP_WRITE_COPY_OR_CONSTANT] = "COPY_OR_CONSTANT",
    [SP_WRITE_LOAD] = "LOAD",
    [SP_WRITE_OTHER] = "OTHER",
};

static const char *const class_names[CLASS_COUNT] = {
    [CLASS_LOAD_8] = "LOAD_8",
    [CLASS_LOAD_16] = "LOAD_16",
    [CLASS_LOAD_32] = "LOAD_32",
    [CLASS_LOAD_64] = "LOAD_64",
    [CLASS_STORE_8] = "STORE_8",
    [CLASS_STORE_16] = "STORE_16",
    [CLASS_STORE_32] = "STORE_32",
    [CLASS_STORE_64] = "STORE_64",
    [CLASS_COPY_IMMEDIATE] = "COPY_IMMEDIATE",
    [CLASS_ADD_SUB] = "ADD_SUB",
    [CLASS_LOGICAL] = "LOGICAL",
    [CLASS_SHIFT] = "SHIFT",
    [CLASS_COMPARE] = "COMPARE",
    [CLASS_CONDITIONAL_BRANCH] = "CONDITIONAL_BRANCH",
    [CLASS_DIRECT_CONTROL] = "DIRECT_CONTROL",
    [CLASS_INDIRECT_CONTROL] = "INDIRECT_CONTROL",
    [CLASS_MULTIPLY] = "MULTIPLY",
    [CLASS_DIVIDE_REMAINDER] = "DIVIDE_REMAINDER",
    [CLASS_ATOMIC] = "ATOMIC",
    [CLASS_FP_MEMORY] = "FP_MEMORY",
    [CLASS_FP_ARITHMETIC] = "FP_ARITHMETIC",
    [CLASS_FENCE] = "FENCE",
    [CLASS_SYSTEM] = "SYSTEM",
    [CLASS_X86_STRING] = "X86_STRING",
    [CLASS_OTHER] = "OTHER",
};

struct execution_counts {
    uint64_t instructions;
    uint64_t classes[CLASS_COUNT];
    uint64_t memory_reads;
    uint64_t memory_writes;
    uint64_t size_2;
    uint64_t size_4;
    uint64_t size_other;
    uint64_t riscv_memory_bases[32];
    uint64_t riscv_memory_base_other;
    uint64_t riscv_sp_writes[SP_WRITE_CLASS_COUNT];
};

// Measure whether a static RV instruction observes one effective stack
// address (or at least one stack page) across the complete workload. A fixed
// open-addressed table keeps the memory callback lock-free; translation-time
// insertion is serialized only while claiming a previously unseen PC. This
// is diagnostic evidence for or against one-entry guarded address
// specialization, never timing evidence.
#define SP_SITE_CAPACITY (1u << 18)

struct sp_memory_site {
    uint64_t pc_key;
    uint64_t first_address;
    uint64_t events;
    uint64_t same_address;
    uint64_t same_page;
    uint64_t page_keys[4];
    uint64_t first_two_page;
    uint64_t first_four_page;
};

static struct sp_memory_site *sp_memory_sites;
static GMutex sp_memory_sites_lock;
static uint64_t sp_recent_pages[2];
static uint64_t sp_recent_page_events;
static uint64_t sp_recent_page_one_hits;
static uint64_t sp_recent_page_two_hits;
static uint64_t sp_current_page_events;
static uint64_t sp_current_page_same;
static uint64_t sp_current_page_below;
static uint64_t sp_current_page_above;
static uint64_t sp_current_page_other;
static uint64_t sp_current_page_read_failures;

static struct qemu_plugin_scoreboard *counts;
static qemu_plugin_u64 instructions;
static qemu_plugin_u64 classes[CLASS_COUNT];
static qemu_plugin_u64 memory_reads;
static qemu_plugin_u64 memory_writes;
static qemu_plugin_u64 size_2;
static qemu_plugin_u64 size_4;
static qemu_plugin_u64 size_other;
static qemu_plugin_u64 riscv_memory_bases[32];
static qemu_plugin_u64 riscv_memory_base_other;
static qemu_plugin_u64 riscv_sp_writes[SP_WRITE_CLASS_COUNT];
static const char *target_name;

// The scorecard's QEMU user-mode diagnostic is single-vCPU, but retain a
// bounded per-vCPU register handle so the plugin fails closed rather than
// silently reading another vCPU if that ever changes.  Register callbacks are
// used only for affine-immediate sp updates, where the post-instruction value
// is exactly old_sp + immediate.
#define MAX_TRACKED_VCPUS 64
static struct qemu_plugin_register *sp_registers[MAX_TRACKED_VCPUS];
static GByteArray *sp_register_values[MAX_TRACKED_VCPUS];
static uint64_t sp_affine_immediate_events;
static uint64_t sp_affine_immediate_same_page;
static uint64_t sp_affine_immediate_cross_page;
static uint64_t sp_affine_immediate_read_failures;
static uint64_t sp_affine_immediate_abs_le_32;
static uint64_t sp_affine_immediate_abs_le_128;
static uint64_t sp_affine_immediate_abs_le_512;

static uint64_t little_endian_u64(const GByteArray *value);

static size_t sp_site_hash(uint64_t pc)
{
    pc ^= pc >> 30;
    pc *= UINT64_C(0xbf58476d1ce4e5b9);
    pc ^= pc >> 27;
    pc *= UINT64_C(0x94d049bb133111eb);
    pc ^= pc >> 31;
    return (size_t)pc & (SP_SITE_CAPACITY - 1);
}

static struct sp_memory_site *sp_site_for_pc(uint64_t pc)
{
    // User-mode instruction address zero is not executable in this workload;
    // retain zero as the empty-slot marker and use pc+1 as the stored key.
    uint64_t key = pc + 1;
    g_mutex_lock(&sp_memory_sites_lock);
    size_t index = sp_site_hash(pc);
    for (size_t probe = 0; probe < SP_SITE_CAPACITY; probe++) {
        struct sp_memory_site *site =
            &sp_memory_sites[(index + probe) & (SP_SITE_CAPACITY - 1)];
        if (site->pc_key == key) {
            g_mutex_unlock(&sp_memory_sites_lock);
            return site;
        }
        if (site->pc_key == 0) {
            site->pc_key = key;
            g_mutex_unlock(&sp_memory_sites_lock);
            return site;
        }
    }
    g_mutex_unlock(&sp_memory_sites_lock);
    return NULL;
}

static void observe_sp_memory(unsigned int vcpu_index,
                              qemu_plugin_meminfo_t info,
                              uint64_t vaddr, void *userdata)
{
    (void)vcpu_index;
    (void)info;
    struct sp_memory_site *site = userdata;
    uint64_t expected = 0;
    if (!__atomic_compare_exchange_n(&site->first_address, &expected, vaddr,
                                     false, __ATOMIC_RELAXED,
                                     __ATOMIC_RELAXED)) {
        expected = __atomic_load_n(&site->first_address, __ATOMIC_RELAXED);
    } else {
        expected = vaddr;
    }
    __atomic_fetch_add(&site->events, 1, __ATOMIC_RELAXED);
    if (vaddr == expected) {
        __atomic_fetch_add(&site->same_address, 1, __ATOMIC_RELAXED);
    }
    if ((vaddr >> 12) == (expected >> 12)) {
        __atomic_fetch_add(&site->same_page, 1, __ATOMIC_RELAXED);
    }
    __atomic_fetch_add(&sp_current_page_events, 1, __ATOMIC_RELAXED);
    if (vcpu_index >= MAX_TRACKED_VCPUS || sp_registers[vcpu_index] == NULL ||
        sp_register_values[vcpu_index] == NULL) {
        __atomic_fetch_add(&sp_current_page_read_failures, 1,
                           __ATOMIC_RELAXED);
    } else {
        GByteArray *value = sp_register_values[vcpu_index];
        g_byte_array_set_size(value, 0);
        if (!qemu_plugin_read_register(sp_registers[vcpu_index], value) ||
            value->len == 0) {
            __atomic_fetch_add(&sp_current_page_read_failures, 1,
                               __ATOMIC_RELAXED);
        } else {
            uint64_t sp_page = little_endian_u64(value) >> 12;
            uint64_t address_page = vaddr >> 12;
            if (address_page == sp_page) {
                __atomic_fetch_add(&sp_current_page_same, 1,
                                   __ATOMIC_RELAXED);
            } else if (address_page + 1 == sp_page) {
                __atomic_fetch_add(&sp_current_page_below, 1,
                                   __ATOMIC_RELAXED);
            } else if (address_page == sp_page + 1) {
                __atomic_fetch_add(&sp_current_page_above, 1,
                                   __ATOMIC_RELAXED);
            } else {
                __atomic_fetch_add(&sp_current_page_other, 1,
                                   __ATOMIC_RELAXED);
            }
        }
    }
    uint64_t page_key = (vaddr >> 12) + 1;
    sp_recent_page_events++;
    if (sp_recent_pages[0] == page_key) {
        sp_recent_page_one_hits++;
        sp_recent_page_two_hits++;
    } else if (sp_recent_pages[1] == page_key) {
        sp_recent_page_two_hits++;
        sp_recent_pages[1] = sp_recent_pages[0];
        sp_recent_pages[0] = page_key;
    } else {
        sp_recent_pages[1] = sp_recent_pages[0];
        sp_recent_pages[0] = page_key;
    }
    int matched_slot = -1;
    for (int slot = 0; slot < 4; slot++) {
        uint64_t observed =
            __atomic_load_n(&site->page_keys[slot], __ATOMIC_RELAXED);
        if (observed == page_key) {
            matched_slot = slot;
            break;
        }
        if (observed == 0 &&
            __atomic_compare_exchange_n(&site->page_keys[slot], &observed,
                                        page_key, false, __ATOMIC_RELAXED,
                                        __ATOMIC_RELAXED)) {
            matched_slot = slot;
            break;
        }
    }
    if (matched_slot >= 0 && matched_slot < 2) {
        __atomic_fetch_add(&site->first_two_page, 1, __ATOMIC_RELAXED);
    }
    if (matched_slot >= 0) {
        __atomic_fetch_add(&site->first_four_page, 1, __ATOMIC_RELAXED);
    }
}

static bool is_one_of(const char *mnemonic, const char *const *values,
                      size_t value_count)
{
    for (size_t i = 0; i < value_count; i++) {
        if (strcmp(mnemonic, values[i]) == 0) {
            return true;
        }
    }
    return false;
}

#define IS_ONE_OF(mnemonic, ...)                                              \
    is_one_of((mnemonic),                                                     \
              (const char *const[]){__VA_ARGS__},                             \
              sizeof((const char *const[]){__VA_ARGS__}) / sizeof(char *))

static enum instruction_class classify_riscv(const char *mnemonic)
{
    if (IS_ONE_OF(mnemonic, "lb", "lbu")) {
        return CLASS_LOAD_8;
    }
    if (IS_ONE_OF(mnemonic, "lh", "lhu")) {
        return CLASS_LOAD_16;
    }
    if (IS_ONE_OF(mnemonic, "lw", "lwu")) {
        return CLASS_LOAD_32;
    }
    if (strcmp(mnemonic, "ld") == 0) {
        return CLASS_LOAD_64;
    }
    if (strcmp(mnemonic, "sb") == 0) {
        return CLASS_STORE_8;
    }
    if (strcmp(mnemonic, "sh") == 0) {
        return CLASS_STORE_16;
    }
    if (strcmp(mnemonic, "sw") == 0) {
        return CLASS_STORE_32;
    }
    if (strcmp(mnemonic, "sd") == 0) {
        return CLASS_STORE_64;
    }
    if (IS_ONE_OF(mnemonic, "flw", "fld", "fsw", "fsd")) {
        return CLASS_FP_MEMORY;
    }
    if (IS_ONE_OF(mnemonic, "mv", "li", "lla", "la", "lui", "auipc",
                  "nop", "sext.w", "zext.w")) {
        return CLASS_COPY_IMMEDIATE;
    }
    if (IS_ONE_OF(mnemonic, "add", "addi", "addw", "addiw", "sub",
                  "subw", "neg", "negw")) {
        return CLASS_ADD_SUB;
    }
    if (IS_ONE_OF(mnemonic, "and", "andi", "or", "ori", "xor", "xori",
                  "not", "orc.b")) {
        return CLASS_LOGICAL;
    }
    if (IS_ONE_OF(mnemonic, "sll", "slli", "sllw", "slliw", "srl",
                  "srli", "srlw", "srliw", "sra", "srai", "sraw",
                  "sraiw", "rol", "rolw", "ror", "rori", "rorw",
                  "roriw")) {
        return CLASS_SHIFT;
    }
    if (IS_ONE_OF(mnemonic, "slt", "slti", "sltu", "sltiu", "seqz",
                  "snez", "sltz", "sgtz", "min", "minu", "max",
                  "maxu")) {
        return CLASS_COMPARE;
    }
    if (IS_ONE_OF(mnemonic, "beq", "bne", "blt", "bge", "bltu", "bgeu",
                  "beqz", "bnez", "bgt", "bgtu", "ble", "bleu",
                  "bgtz", "blez", "bgez", "bltz")) {
        return CLASS_CONDITIONAL_BRANCH;
    }
    if (IS_ONE_OF(mnemonic, "jal", "j", "call", "tail")) {
        return CLASS_DIRECT_CONTROL;
    }
    if (IS_ONE_OF(mnemonic, "jalr", "jr", "ret")) {
        return CLASS_INDIRECT_CONTROL;
    }
    if (strncmp(mnemonic, "mul", 3) == 0) {
        return CLASS_MULTIPLY;
    }
    if (strncmp(mnemonic, "div", 3) == 0 || strncmp(mnemonic, "rem", 3) == 0) {
        return CLASS_DIVIDE_REMAINDER;
    }
    if (strncmp(mnemonic, "amo", 3) == 0 || strncmp(mnemonic, "lr.", 3) == 0 ||
        strncmp(mnemonic, "sc.", 3) == 0) {
        return CLASS_ATOMIC;
    }
    if (strcmp(mnemonic, "fence") == 0 || strcmp(mnemonic, "fence.i") == 0 ||
        strcmp(mnemonic, "pause") == 0) {
        return CLASS_FENCE;
    }
    if (mnemonic[0] == 'f') {
        return CLASS_FP_ARITHMETIC;
    }
    if (IS_ONE_OF(mnemonic, "ecall", "ebreak", "unimp", "wfi", "mret",
                  "sret") ||
        strncmp(mnemonic, "csr", 3) == 0) {
        return CLASS_SYSTEM;
    }
    return CLASS_OTHER;
}

static bool x86_stem(const char *mnemonic, const char *stem)
{
    size_t len = strlen(stem);
    if (strncmp(mnemonic, stem, len) != 0) {
        return false;
    }
    const char *suffix = mnemonic + len;
    return suffix[0] == '\0' ||
           (suffix[1] == '\0' && strchr("bwlq", suffix[0]) != NULL);
}

static enum instruction_class classify_x86(const char *mnemonic)
{
    if (x86_stem(mnemonic, "add") || x86_stem(mnemonic, "adc") ||
        x86_stem(mnemonic, "sub") || x86_stem(mnemonic, "sbb") ||
        x86_stem(mnemonic, "inc") || x86_stem(mnemonic, "dec") ||
        x86_stem(mnemonic, "neg")) {
        return CLASS_ADD_SUB;
    }
    if (x86_stem(mnemonic, "and") || x86_stem(mnemonic, "or") ||
        x86_stem(mnemonic, "xor") || x86_stem(mnemonic, "not")) {
        return CLASS_LOGICAL;
    }
    if (x86_stem(mnemonic, "shl") || x86_stem(mnemonic, "sal") ||
        x86_stem(mnemonic, "shr") || x86_stem(mnemonic, "sar") ||
        x86_stem(mnemonic, "rol") || x86_stem(mnemonic, "ror") ||
        x86_stem(mnemonic, "shld") || x86_stem(mnemonic, "shrd")) {
        return CLASS_SHIFT;
    }
    if (x86_stem(mnemonic, "cmp") || x86_stem(mnemonic, "test") ||
        strncmp(mnemonic, "set", 3) == 0) {
        return CLASS_COMPARE;
    }
    if (mnemonic[0] == 'j' && strncmp(mnemonic, "jmp", 3) != 0) {
        return CLASS_CONDITIONAL_BRANCH;
    }
    if (strncmp(mnemonic, "call", 4) == 0) {
        return CLASS_DIRECT_CONTROL;
    }
    if (strncmp(mnemonic, "jmp", 3) == 0 || strncmp(mnemonic, "ret", 3) == 0 ||
        strncmp(mnemonic, "iret", 4) == 0) {
        return CLASS_INDIRECT_CONTROL;
    }
    if (x86_stem(mnemonic, "mul") || x86_stem(mnemonic, "imul")) {
        return CLASS_MULTIPLY;
    }
    if (x86_stem(mnemonic, "div") || x86_stem(mnemonic, "idiv")) {
        return CLASS_DIVIDE_REMAINDER;
    }
    if (strncmp(mnemonic, "lock", 4) == 0 ||
        x86_stem(mnemonic, "cmpxchg") || x86_stem(mnemonic, "xadd")) {
        return CLASS_ATOMIC;
    }
    if (x86_stem(mnemonic, "mov") || strncmp(mnemonic, "movz", 4) == 0 ||
        strncmp(mnemonic, "movs", 4) == 0 || x86_stem(mnemonic, "lea") ||
        x86_stem(mnemonic, "push") || x86_stem(mnemonic, "pop") ||
        x86_stem(mnemonic, "xchg")) {
        return CLASS_COPY_IMMEDIATE;
    }
    if (strncmp(mnemonic, "fld", 3) == 0 || strncmp(mnemonic, "fst", 3) == 0) {
        return CLASS_FP_MEMORY;
    }
    if (mnemonic[0] == 'f' || strncmp(mnemonic, "xmm", 3) == 0) {
        return CLASS_FP_ARITHMETIC;
    }
    if (strncmp(mnemonic, "lods", 4) == 0 || strncmp(mnemonic, "stos", 4) == 0 ||
        strncmp(mnemonic, "scas", 4) == 0 || strncmp(mnemonic, "cmps", 4) == 0 ||
        strncmp(mnemonic, "ins", 3) == 0 || strncmp(mnemonic, "outs", 4) == 0) {
        return CLASS_X86_STRING;
    }
    if (strncmp(mnemonic, "sys", 3) == 0 || strncmp(mnemonic, "int", 3) == 0 ||
        strcmp(mnemonic, "ud2") == 0 || strcmp(mnemonic, "hlt") == 0) {
        return CLASS_SYSTEM;
    }
    if (strncmp(mnemonic, "mfence", 6) == 0 || strncmp(mnemonic, "sfence", 6) == 0 ||
        strncmp(mnemonic, "lfence", 6) == 0) {
        return CLASS_FENCE;
    }
    return CLASS_OTHER;
}

static void extract_mnemonic(char *out, size_t out_size, const char *disassembly)
{
    while (isspace((unsigned char)*disassembly)) {
        disassembly++;
    }
    size_t len = 0;
    while (disassembly[len] != '\0' &&
           !isspace((unsigned char)disassembly[len]) && len + 1 < out_size) {
        out[len] = (char)tolower((unsigned char)disassembly[len]);
        len++;
    }
    out[len] = '\0';

    // QEMU prints x86 repeat/lock prefixes as the first token. Classify the
    // underlying operation, except for lock which remains an atomic marker.
    if (strcmp(out, "rep") == 0 || strcmp(out, "repe") == 0 ||
        strcmp(out, "repz") == 0 || strcmp(out, "repne") == 0 ||
        strcmp(out, "repnz") == 0) {
        disassembly += len;
        while (isspace((unsigned char)*disassembly)) {
            disassembly++;
        }
        len = 0;
        while (disassembly[len] != '\0' &&
               !isspace((unsigned char)disassembly[len]) && len + 1 < out_size) {
            out[len] = (char)tolower((unsigned char)disassembly[len]);
            len++;
        }
        out[len] = '\0';
    }
}

static enum instruction_class classify_instruction(const char *mnemonic)
{
    if (strncmp(target_name, "riscv", 5) == 0) {
        return classify_riscv(mnemonic);
    }
    if (strstr(target_name, "i386") != NULL || strstr(target_name, "x86") != NULL) {
        return classify_x86(mnemonic);
    }
    return CLASS_OTHER;
}

static int riscv_register_index(const char *name, size_t length)
{
    static const char *const abi_names[32] = {
        "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2",
        "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5",
        "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7",
        "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
    };
    if (length == 2 && name[0] == 'f' && name[1] == 'p') {
        return 8;
    }
    if (length >= 2 && length <= 3 && name[0] == 'x') {
        unsigned int value = 0;
        for (size_t i = 1; i < length; i++) {
            if (!isdigit((unsigned char)name[i])) {
                return -1;
            }
            value = value * 10 + (unsigned int)(name[i] - '0');
        }
        return value < 32 ? (int)value : -1;
    }
    for (size_t i = 0; i < 32; i++) {
        if (strlen(abi_names[i]) == length &&
            strncmp(name, abi_names[i], length) == 0) {
            return (int)i;
        }
    }
    return -1;
}

static size_t riscv_operands(const char *disassembly,
                             char operands[][32], size_t capacity)
{
    const char *cursor = disassembly;
    while (*cursor != '\0' && !isspace((unsigned char)*cursor)) {
        cursor++;
    }
    size_t count = 0;
    while (*cursor != '\0' && count < capacity) {
        while (isspace((unsigned char)*cursor) || *cursor == ',') {
            cursor++;
        }
        if (*cursor == '\0') {
            break;
        }
        size_t length = 0;
        while (cursor[length] != '\0' && cursor[length] != ',' &&
               !isspace((unsigned char)cursor[length])) {
            if (length + 1 < sizeof(operands[0])) {
                operands[count][length] =
                    (char)tolower((unsigned char)cursor[length]);
            }
            length++;
        }
        size_t stored = length < sizeof(operands[0]) - 1
                            ? length
                            : sizeof(operands[0]) - 1;
        operands[count][stored] = '\0';
        count++;
        cursor += length;
    }
    return count;
}

static int operand_register(const char *operand)
{
    return riscv_register_index(operand, strlen(operand));
}

static bool parse_immediate(const char *operand, int64_t *value)
{
    char *end = NULL;
    gint64 parsed = g_ascii_strtoll(operand, &end, 0);
    if (end == operand || *end != '\0') {
        return false;
    }
    *value = (int64_t)parsed;
    return true;
}

static bool class_writes_first_integer_operand(enum instruction_class class_id,
                                                const char *mnemonic)
{
    switch (class_id) {
    case CLASS_LOAD_8:
    case CLASS_LOAD_16:
    case CLASS_LOAD_32:
    case CLASS_LOAD_64:
    case CLASS_COPY_IMMEDIATE:
    case CLASS_ADD_SUB:
    case CLASS_LOGICAL:
    case CLASS_SHIFT:
    case CLASS_COMPARE:
    case CLASS_MULTIPLY:
    case CLASS_DIVIDE_REMAINDER:
    case CLASS_ATOMIC:
        return true;
    case CLASS_DIRECT_CONTROL:
        // jal with an explicit rd writes it; j/call/tail use fixed aliases.
        return strcmp(mnemonic, "jal") == 0;
    case CLASS_INDIRECT_CONTROL:
        // jalr with an explicit rd writes it; jr/ret do not.
        return strcmp(mnemonic, "jalr") == 0;
    case CLASS_SYSTEM:
        // CSR read/modify/write instructions can name an integer rd.  Pure
        // write aliases (csrw/csrwi/...) have a CSR name as operand zero and
        // therefore cannot be mistaken for sp by the operand parser.
        return strncmp(mnemonic, "csr", 3) == 0;
    case CLASS_FP_ARITHMETIC:
        // Only FP-to-integer conversion/move/classify operations can name sp.
        return strncmp(mnemonic, "fcvt.", 5) == 0 ||
               strncmp(mnemonic, "fmv.x", 5) == 0 ||
               strncmp(mnemonic, "fclass", 6) == 0;
    default:
        return false;
    }
}

static bool classify_sp_write(const char *mnemonic, const char *disassembly,
                              enum instruction_class class_id,
                              enum sp_write_class *write_class,
                              int64_t *affine_immediate)
{
    char operand[4][32] = {{0}};
    size_t count = riscv_operands(disassembly, operand, G_N_ELEMENTS(operand));
    if (count == 0 || operand_register(operand[0]) != 2 ||
        !class_writes_first_integer_operand(class_id, mnemonic)) {
        return false;
    }

    if ((strcmp(mnemonic, "addi") == 0 || strcmp(mnemonic, "addiw") == 0) &&
        count >= 3 && operand_register(operand[1]) == 2 &&
        parse_immediate(operand[2], affine_immediate)) {
        *write_class = SP_WRITE_AFFINE_IMMEDIATE;
        return true;
    }
    if ((strcmp(mnemonic, "add") == 0 || strcmp(mnemonic, "addw") == 0) &&
        count >= 3 &&
        (operand_register(operand[1]) == 2 || operand_register(operand[2]) == 2)) {
        *write_class = SP_WRITE_AFFINE_REGISTER;
        return true;
    }
    if ((strcmp(mnemonic, "sub") == 0 || strcmp(mnemonic, "subw") == 0) &&
        count >= 3 && operand_register(operand[1]) == 2) {
        *write_class = SP_WRITE_AFFINE_REGISTER;
        return true;
    }
    if (class_id >= CLASS_LOAD_8 && class_id <= CLASS_LOAD_64) {
        *write_class = SP_WRITE_LOAD;
        return true;
    }
    if (class_id == CLASS_COPY_IMMEDIATE ||
        ((strcmp(mnemonic, "addi") == 0 || strcmp(mnemonic, "addiw") == 0) &&
         count >= 3)) {
        *write_class = SP_WRITE_COPY_OR_CONSTANT;
        return true;
    }
    *write_class = SP_WRITE_OTHER;
    return true;
}

static uint64_t little_endian_u64(const GByteArray *value)
{
    uint64_t result = 0;
    size_t length = value->len < sizeof(result) ? value->len : sizeof(result);
    for (size_t i = 0; i < length; i++) {
        result |= (uint64_t)value->data[i] << (i * 8);
    }
    return result;
}

static void observe_affine_sp_update(unsigned int vcpu_index, void *userdata)
{
    int64_t immediate = (int64_t)GPOINTER_TO_INT(userdata);
    __atomic_fetch_add(&sp_affine_immediate_events, 1, __ATOMIC_RELAXED);
    uint64_t magnitude = immediate < 0 ? (uint64_t)(-immediate)
                                       : (uint64_t)immediate;
    if (magnitude <= 32) {
        __atomic_fetch_add(&sp_affine_immediate_abs_le_32, 1,
                           __ATOMIC_RELAXED);
    }
    if (magnitude <= 128) {
        __atomic_fetch_add(&sp_affine_immediate_abs_le_128, 1,
                           __ATOMIC_RELAXED);
    }
    if (magnitude <= 512) {
        __atomic_fetch_add(&sp_affine_immediate_abs_le_512, 1,
                           __ATOMIC_RELAXED);
    }
    if (vcpu_index >= MAX_TRACKED_VCPUS || sp_registers[vcpu_index] == NULL ||
        sp_register_values[vcpu_index] == NULL) {
        __atomic_fetch_add(&sp_affine_immediate_read_failures, 1,
                           __ATOMIC_RELAXED);
        return;
    }
    GByteArray *value = sp_register_values[vcpu_index];
    g_byte_array_set_size(value, 0);
    if (!qemu_plugin_read_register(sp_registers[vcpu_index], value) ||
        value->len == 0) {
        __atomic_fetch_add(&sp_affine_immediate_read_failures, 1,
                           __ATOMIC_RELAXED);
        return;
    }
    uint64_t old_sp = little_endian_u64(value);
    uint64_t new_sp = old_sp + (uint64_t)immediate;
    if ((old_sp >> 12) == (new_sp >> 12)) {
        __atomic_fetch_add(&sp_affine_immediate_same_page, 1,
                           __ATOMIC_RELAXED);
    } else {
        __atomic_fetch_add(&sp_affine_immediate_cross_page, 1,
                           __ATOMIC_RELAXED);
    }
}

static void vcpu_init(qemu_plugin_id_t id, unsigned int vcpu_index)
{
    (void)id;
    if (vcpu_index >= MAX_TRACKED_VCPUS ||
        strncmp(target_name, "riscv", 5) != 0) {
        return;
    }
    GArray *registers = qemu_plugin_get_registers();
    if (registers == NULL) {
        return;
    }
    for (guint i = 0; i < registers->len; i++) {
        qemu_plugin_reg_descriptor *descriptor = &g_array_index(
            registers, qemu_plugin_reg_descriptor, i);
        if (strcmp(descriptor->name, "sp") == 0) {
            sp_registers[vcpu_index] = descriptor->handle;
            sp_register_values[vcpu_index] = g_byte_array_new();
            break;
        }
    }
    g_array_free(registers, true);
}

static int riscv_memory_base(const char *disassembly)
{
    const char *open = strrchr(disassembly, '(');
    if (open == NULL) {
        return -1;
    }
    const char *close = strchr(open + 1, ')');
    if (close == NULL || close == open + 1) {
        return -1;
    }
    return riscv_register_index(open + 1, (size_t)(close - open - 1));
}

static bool is_memory_class(enum instruction_class class_id)
{
    return (class_id >= CLASS_LOAD_8 && class_id <= CLASS_STORE_64) ||
           class_id == CLASS_ATOMIC || class_id == CLASS_FP_MEMORY;
}

static void translate_block(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    (void)id;
    qemu_plugin_register_vcpu_tb_exec_inline_per_vcpu(
        tb,
        QEMU_PLUGIN_INLINE_ADD_U64,
        instructions,
        qemu_plugin_tb_n_insns(tb));

    size_t instruction_count = qemu_plugin_tb_n_insns(tb);
    for (size_t i = 0; i < instruction_count; i++) {
        struct qemu_plugin_insn *insn = qemu_plugin_tb_get_insn(tb, i);
        char *disassembly = qemu_plugin_insn_disas(insn);
        char mnemonic[32];
        extract_mnemonic(mnemonic, sizeof(mnemonic), disassembly);

        enum instruction_class class_id = classify_instruction(mnemonic);
        if (strncmp(target_name, "riscv", 5) == 0) {
            if (is_memory_class(class_id)) {
                int base = riscv_memory_base(disassembly);
                qemu_plugin_register_vcpu_insn_exec_inline_per_vcpu(
                    insn, QEMU_PLUGIN_INLINE_ADD_U64,
                    base >= 0 ? riscv_memory_bases[base] : riscv_memory_base_other,
                    1);
                if (base == 2 && sp_memory_sites != NULL) {
                    struct sp_memory_site *site =
                        sp_site_for_pc(qemu_plugin_insn_vaddr(insn));
                    if (site != NULL) {
                    qemu_plugin_register_vcpu_mem_cb(
                            insn, observe_sp_memory, QEMU_PLUGIN_CB_R_REGS,
                            QEMU_PLUGIN_MEM_RW, site);
                    }
                }
            }
            enum sp_write_class write_class;
            int64_t affine_immediate = 0;
            if (classify_sp_write(mnemonic, disassembly, class_id,
                                  &write_class, &affine_immediate)) {
                qemu_plugin_register_vcpu_insn_exec_inline_per_vcpu(
                    insn, QEMU_PLUGIN_INLINE_ADD_U64,
                    riscv_sp_writes[write_class], 1);
                if (write_class == SP_WRITE_AFFINE_IMMEDIATE) {
                    qemu_plugin_register_vcpu_insn_exec_cb(
                        insn, observe_affine_sp_update,
                        QEMU_PLUGIN_CB_R_REGS,
                        GINT_TO_POINTER((gint)affine_immediate));
                }
            }
        }
        g_free(disassembly);
        qemu_plugin_register_vcpu_insn_exec_inline_per_vcpu(
            insn, QEMU_PLUGIN_INLINE_ADD_U64, classes[class_id], 1);
        qemu_plugin_register_vcpu_mem_inline_per_vcpu(
            insn, QEMU_PLUGIN_MEM_R, QEMU_PLUGIN_INLINE_ADD_U64,
            memory_reads, 1);
        qemu_plugin_register_vcpu_mem_inline_per_vcpu(
            insn, QEMU_PLUGIN_MEM_W, QEMU_PLUGIN_INLINE_ADD_U64,
            memory_writes, 1);

        qemu_plugin_u64 size_counter = size_other;
        if (qemu_plugin_insn_size(insn) == 2) {
            size_counter = size_2;
        } else if (qemu_plugin_insn_size(insn) == 4) {
            size_counter = size_4;
        }
        qemu_plugin_register_vcpu_insn_exec_inline_per_vcpu(
            insn, QEMU_PLUGIN_INLINE_ADD_U64, size_counter, 1);
    }
}

static void output_count(const char *name, uint64_t value)
{
    char *line = g_strdup_printf("%s=%" PRIu64 "\n", name, value);
    qemu_plugin_outs(line);
    g_free(line);
}

static void report(qemu_plugin_id_t id, void *userdata)
{
    (void)id;
    (void)userdata;
    char line[128];
    snprintf(line, sizeof(line), "QEMU_TARGET=%s\n", target_name);
    qemu_plugin_outs(line);
    output_count("QEMU_GUEST_INSTRUCTIONS", qemu_plugin_u64_sum(instructions));
    output_count("QEMU_GUEST_MEMORY_READS", qemu_plugin_u64_sum(memory_reads));
    output_count("QEMU_GUEST_MEMORY_WRITES", qemu_plugin_u64_sum(memory_writes));
    output_count("QEMU_GUEST_SIZE_2", qemu_plugin_u64_sum(size_2));
    output_count("QEMU_GUEST_SIZE_4", qemu_plugin_u64_sum(size_4));
    output_count("QEMU_GUEST_SIZE_OTHER", qemu_plugin_u64_sum(size_other));

    uint64_t classified = 0;
    for (size_t i = 0; i < CLASS_COUNT; i++) {
        uint64_t value = qemu_plugin_u64_sum(classes[i]);
        classified += value;
        snprintf(line, sizeof(line), "QEMU_GUEST_CLASS_%s", class_names[i]);
        output_count(line, value);
    }
    output_count("QEMU_GUEST_CLASSIFIED", classified);
    if (strncmp(target_name, "riscv", 5) == 0) {
        for (size_t i = 0; i < 32; i++) {
            snprintf(line, sizeof(line), "QEMU_GUEST_RV_MEMORY_BASE_X%zu", i);
            output_count(line, qemu_plugin_u64_sum(riscv_memory_bases[i]));
        }
        output_count("QEMU_GUEST_RV_MEMORY_BASE_OTHER",
                     qemu_plugin_u64_sum(riscv_memory_base_other));
        uint64_t sp_writes = 0;
        for (size_t i = 0; i < SP_WRITE_CLASS_COUNT; i++) {
            uint64_t value = qemu_plugin_u64_sum(riscv_sp_writes[i]);
            sp_writes += value;
            snprintf(line, sizeof(line), "QEMU_GUEST_RV_SP_WRITE_%s",
                     sp_write_class_names[i]);
            output_count(line, value);
        }
        output_count("QEMU_GUEST_RV_SP_WRITES", sp_writes);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_EVENTS",
                     sp_affine_immediate_events);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_SAME_PAGE",
                     sp_affine_immediate_same_page);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_CROSS_PAGE",
                     sp_affine_immediate_cross_page);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_READ_FAILURES",
                     sp_affine_immediate_read_failures);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_ABS_LE_32",
                     sp_affine_immediate_abs_le_32);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_ABS_LE_128",
                     sp_affine_immediate_abs_le_128);
        output_count("QEMU_GUEST_RV_SP_AFFINE_IMMEDIATE_ABS_LE_512",
                     sp_affine_immediate_abs_le_512);
        uint64_t sp_events = 0;
        uint64_t sp_same_address = 0;
        uint64_t sp_same_page = 0;
        uint64_t sp_first_two_page = 0;
        uint64_t sp_first_four_page = 0;
        uint64_t sp_sites = 0;
        uint64_t sp_unstable_address_sites = 0;
        uint64_t sp_unstable_page_sites = 0;
        for (size_t i = 0; i < SP_SITE_CAPACITY; i++) {
            struct sp_memory_site *site = &sp_memory_sites[i];
            uint64_t events =
                __atomic_load_n(&site->events, __ATOMIC_RELAXED);
            if (events == 0) {
                continue;
            }
            uint64_t same_address =
                __atomic_load_n(&site->same_address, __ATOMIC_RELAXED);
            uint64_t same_page =
                __atomic_load_n(&site->same_page, __ATOMIC_RELAXED);
            uint64_t first_two_page =
                __atomic_load_n(&site->first_two_page, __ATOMIC_RELAXED);
            uint64_t first_four_page =
                __atomic_load_n(&site->first_four_page, __ATOMIC_RELAXED);
            sp_events += events;
            sp_same_address += same_address;
            sp_same_page += same_page;
            sp_first_two_page += first_two_page;
            sp_first_four_page += first_four_page;
            sp_sites++;
            sp_unstable_address_sites += same_address != events;
            sp_unstable_page_sites += same_page != events;
        }
        output_count("QEMU_GUEST_RV_SP_ADDRESS_EVENTS", sp_events);
        output_count("QEMU_GUEST_RV_SP_SAME_FIRST_ADDRESS", sp_same_address);
        output_count("QEMU_GUEST_RV_SP_SAME_FIRST_PAGE", sp_same_page);
        output_count("QEMU_GUEST_RV_SP_FIRST_TWO_PAGE", sp_first_two_page);
        output_count("QEMU_GUEST_RV_SP_FIRST_FOUR_PAGE", sp_first_four_page);
        output_count("QEMU_GUEST_RV_SP_ADDRESS_SITES", sp_sites);
        output_count("QEMU_GUEST_RV_SP_UNSTABLE_ADDRESS_SITES",
                     sp_unstable_address_sites);
        output_count("QEMU_GUEST_RV_SP_UNSTABLE_PAGE_SITES",
                     sp_unstable_page_sites);
        output_count("QEMU_GUEST_RV_SP_RECENT_PAGE_EVENTS",
                     sp_recent_page_events);
        output_count("QEMU_GUEST_RV_SP_RECENT_ONE_HITS",
                     sp_recent_page_one_hits);
        output_count("QEMU_GUEST_RV_SP_RECENT_TWO_HITS",
                     sp_recent_page_two_hits);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_EVENTS",
                     sp_current_page_events);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_SAME",
                     sp_current_page_same);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_BELOW",
                     sp_current_page_below);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_ABOVE",
                     sp_current_page_above);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_OTHER",
                     sp_current_page_other);
        output_count("QEMU_GUEST_RV_SP_CURRENT_PAGE_READ_FAILURES",
                     sp_current_page_read_failures);
    }
    for (size_t i = 0; i < MAX_TRACKED_VCPUS; i++) {
        if (sp_register_values[i] != NULL) {
            g_byte_array_unref(sp_register_values[i]);
            sp_register_values[i] = NULL;
        }
    }
    g_free(sp_memory_sites);
    qemu_plugin_scoreboard_free(counts);
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id,
                                           const qemu_info_t *info,
                                           int argc, char **argv)
{
    (void)argc;
    (void)argv;
    target_name = info->target_name;
    if (strncmp(target_name, "riscv", 5) == 0) {
        sp_memory_sites = g_new0(struct sp_memory_site, SP_SITE_CAPACITY);
    }
    counts = qemu_plugin_scoreboard_new(sizeof(struct execution_counts));
    instructions = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, instructions);
    for (size_t i = 0; i < CLASS_COUNT; i++) {
        classes[i] = (qemu_plugin_u64){
            counts,
            offsetof(struct execution_counts, classes) + i * sizeof(uint64_t),
        };
    }
    memory_reads = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, memory_reads);
    memory_writes = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, memory_writes);
    size_2 = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, size_2);
    size_4 = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, size_4);
    size_other = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, size_other);
    for (size_t i = 0; i < 32; i++) {
        riscv_memory_bases[i] = (qemu_plugin_u64){
            counts,
            offsetof(struct execution_counts, riscv_memory_bases) +
                i * sizeof(uint64_t),
        };
    }
    riscv_memory_base_other = qemu_plugin_scoreboard_u64_in_struct(
        counts, struct execution_counts, riscv_memory_base_other);
    for (size_t i = 0; i < SP_WRITE_CLASS_COUNT; i++) {
        riscv_sp_writes[i] = (qemu_plugin_u64){
            counts,
            offsetof(struct execution_counts, riscv_sp_writes) +
                i * sizeof(uint64_t),
        };
    }
    qemu_plugin_register_vcpu_init_cb(id, vcpu_init);
    qemu_plugin_register_vcpu_tb_trans_cb(id, translate_block);
    qemu_plugin_register_atexit_cb(id, report, NULL);
    return 0;
}
