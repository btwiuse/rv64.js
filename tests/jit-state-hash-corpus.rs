//! R084 proof-only corpus for the integer-key state tables used by the JIT.
//!
//! Build directly as a `wasm32-unknown-unknown` cdylib.  This deliberately
//! does not link into rv64-wasm: it establishes the local operation bound
//! before a product candidate is admitted.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::hint::black_box;

const MIX: u64 = 0xd6e8_feb8_6659_fd93;
const STEP: u64 = 0x9e37_79b9_7f4a_7c15;

#[inline(always)]
fn avalanche(mut value: u64) -> u64 {
    // Jon Maiga's mx3 finalizer. MIX is odd, so this is a permutation over
    // u64: a single u64 key has no full-width collisions. A per-map seed in
    // the product form prevents trivially transferable bucket attacks.
    value ^= value >> 32;
    value = value.wrapping_mul(MIX);
    value ^= value >> 32;
    value = value.wrapping_mul(MIX);
    value ^ (value >> 32)
}

#[derive(Clone, Copy)]
struct FastBuildHasher {
    seed: u64,
}

impl Default for FastBuildHasher {
    fn default() -> Self {
        Self {
            seed: 0x243f_6a88_85a3_08d3,
        }
    }
}

struct FastHasher {
    state: u64,
}

impl BuildHasher for FastBuildHasher {
    type Hasher = FastHasher;

    #[inline(always)]
    fn build_hasher(&self) -> Self::Hasher {
        FastHasher { state: self.seed }
    }
}

impl FastHasher {
    #[inline(always)]
    fn word(&mut self, value: u64) {
        self.state = avalanche(self.state ^ value.wrapping_add(STEP));
    }
}

impl Hasher for FastHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.state
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.word(u64::from_ne_bytes(chunk.try_into().unwrap()));
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            let mut word = [0_u8; 8];
            word[..tail.len()].copy_from_slice(tail);
            self.word(u64::from_ne_bytes(word) ^ ((tail.len() as u64) << 56));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, value: u8) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u16(&mut self, value: u16) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u32(&mut self, value: u32) {
        self.word(value.into());
    }
    #[inline(always)]
    fn write_u64(&mut self, value: u64) {
        self.word(value);
    }
    #[inline(always)]
    fn write_usize(&mut self, value: usize) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i8(&mut self, value: i8) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i16(&mut self, value: i16) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i32(&mut self, value: i32) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_i64(&mut self, value: i64) {
        self.word(value as u64);
    }
    #[inline(always)]
    fn write_isize(&mut self, value: isize) {
        self.word(value as u64);
    }
}

#[inline(always)]
fn page_key(index: u64) -> (u64, u64) {
    let virtual_page = 0xffff_ffc0_0000_0000_u64
        .wrapping_add((index.wrapping_mul(0x9e37) & 0x3f_ffff) << 12);
    let physical_page = 0x8000_0000_u64
        .wrapping_add((index.wrapping_mul(0x45d9_f3b) & 0x1f_ffff) << 12);
    (virtual_page, physical_page)
}

fn hash_only<S: BuildHasher + Default>(rounds: u32) -> u64 {
    let state = S::default();
    let mut checksum = 0_u64;
    for index in 0..u64::from(rounds) {
        let key = black_box(page_key(index));
        checksum = checksum.rotate_left(7) ^ state.hash_one(key);
    }
    black_box(checksum)
}

fn state_maps<S: BuildHasher + Clone + Default>(entries: u32, rounds: u32) -> u64 {
    let entries = entries.max(1);
    let state = S::default();
    let capacity = entries as usize;
    let mut heat = HashMap::with_capacity_and_hasher(capacity, state.clone());
    let mut last = HashMap::with_capacity_and_hasher(capacity, state.clone());
    let mut mappings = HashMap::with_capacity_and_hasher(capacity, state.clone());
    let mut attempted = HashMap::with_capacity_and_hasher(capacity, state.clone());
    let mut queued = HashSet::with_capacity_and_hasher(capacity, state.clone());
    let mut installed = HashSet::with_capacity_and_hasher(capacity, state);

    for index in 0..u64::from(entries) {
        let key = page_key(index);
        heat.insert(key, index.wrapping_mul(17));
        last.insert(key, index);
        mappings.insert((index.rotate_left(13), key.0), key.1);
        attempted.insert(key, index as u32);
        if index & 3 == 0 {
            queued.insert(key);
        }
        if index & 7 == 0 {
            installed.insert(key);
        }
    }

    let mut checksum = 0_u64;
    for tick in 0..u64::from(rounds) {
        // Odd multiplication plus modulo exercises hits in a stable but
        // non-sequential order. Every sixteenth access is a miss, matching the
        // mix of state probes and insertions in page_policy_observe.
        let hit = tick.wrapping_mul(0x9e37_79b1) % u64::from(entries);
        let index = if tick & 15 == 0 {
            u64::from(entries).wrapping_add(tick)
        } else {
            hit
        };
        let key = black_box(page_key(index));

        let value = heat.entry(key).or_insert(0);
        *value = value.wrapping_add((tick & 0xff) + 1);
        checksum ^= *value;
        checksum = checksum.wrapping_add(last.get(&key).copied().unwrap_or(u64::MAX));
        last.insert(key, tick);
        checksum ^= mappings
            .get(&(hit.rotate_left(13), page_key(hit).0))
            .copied()
            .unwrap_or(0);
        checksum ^= u64::from(attempted.contains_key(&key));
        checksum ^= u64::from(queued.contains(&key)) << 1;
        checksum ^= u64::from(installed.contains(&key)) << 2;
        if tick & 63 == 0 {
            attempted.insert(key, tick as u32);
            queued.insert(key);
        }
    }
    black_box(checksum)
}

#[no_mangle]
pub extern "C" fn run_hash(variant: u32, rounds: u32) -> u64 {
    match variant {
        0 => hash_only::<RandomState>(rounds),
        1 => hash_only::<FastBuildHasher>(rounds),
        _ => u64::MAX,
    }
}

#[no_mangle]
pub extern "C" fn run_maps(variant: u32, entries: u32, rounds: u32) -> u64 {
    match variant {
        0 => state_maps::<RandomState>(entries, rounds),
        1 => state_maps::<FastBuildHasher>(entries, rounds),
        _ => u64::MAX,
    }
}
