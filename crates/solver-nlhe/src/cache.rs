//! Flop/preflop cache: precomputed subgame solutions keyed by canonical
//! spot descriptor.
//!
//! Format:
//! - On disk: packed binary, mmap-friendly
//! - In memory: `HashMap<CacheKey, CachedStrategy>`
//!
//! Populated offline by Colab precompute; consumed at runtime by the
//! live solver as a cache-hit fast path.

// TODO (Day 5, agent A2): implement key type and lookup API.
//
// pub struct CacheKey {
//     canonical_board: [u8; 32],    // hash of canonicalized board
//     spr_bucket: u8,
//     pot_type: PotType,            // SRP | ThreeBetPot | FourBetPot
//     bet_tree_version: u8,
// }
//
// pub struct CachedStrategy { ... }
//
// pub struct Cache {
//     map: HashMap<CacheKey, CachedStrategy>,
// }
//
// impl Cache {
//     pub fn load_from_file(path: &Path) -> Result<Self>
//     pub fn lookup(&self, key: &CacheKey) -> Option<&CachedStrategy>
// }
