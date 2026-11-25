/// Longest regex pattern accepted, in characters (both `/pattern/` literals and `$regex`).
pub const MAX_REGEX_PATTERN_LEN: usize = 256;
/// Most `/pattern/` literals in one editor's text.
pub const MAX_REGEX_LITERALS: usize = 20;
/// Most variables (`var x = ...`) in one script, the largest one, and how much all the places that use a variable may
/// add up to (a variable used in another variable can multiply: this keeps that from being a way to blow up memory).
pub const MAX_SCRIPT_VARIABLES: usize = 50;
pub const MAX_VARIABLE_BYTES: usize = 1024 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 8 * 1024 * 1024;

/// Hard limits applied to every run. Shown to users in the "About" panel and served at
/// `GET /api/v1/limits`.
///
/// There is deliberately no cap on the number of results: the query time limit, the wall
/// clock, and the size of the seeded data bound the work instead.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_collections: usize,
    pub max_docs_per_collection: usize,
    /// Uncompressed size of the request's documents plus query. Clients may send the request
    /// compressed (`Content-Encoding`); this limit applies after decompression.
    pub max_payload_bytes: usize,
    /// Server-side `maxTimeMS` for the query itself.
    pub max_time_ms: u64,
    /// Whole-run budget (seeding, query, result transfer) enforced by the runner.
    pub wall_clock_ms: u64,
    pub max_depth: usize,
    /// User-declared indexes per collection. The automatic `_id` index does not count.
    pub max_indexes_per_collection: usize,
    pub max_index_key_fields: usize,
    pub max_regex_pattern_len: usize,
    /// Ceiling on the total size of one statement's results. There is no cap on the *number* of
    /// results, but an aggregation can multiply data (`$unwind` of `$range`), so the bytes are bounded.
    pub max_result_bytes: usize,
    /// Largest playground that can be saved as a share link.
    pub max_share_bytes: usize,
    /// Whole-run budget when a search index is built (index builds take seconds).
    pub search_wall_clock_ms: u64,
    /// Simultaneous runs that build a search index, so one busy moment cannot starve the cluster.
    pub max_concurrent_search_runs: usize,
    /// Sample-data reads return at most this many documents (the sample collections are large).
    pub sample_max_results: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_collections: 8,
            max_docs_per_collection: 256,
            max_payload_bytes: 32 * 1024 * 1024,
            max_time_ms: 10_000,
            wall_clock_ms: 15_000,
            max_depth: 32,
            max_indexes_per_collection: 3,
            max_index_key_fields: 4,
            max_regex_pattern_len: MAX_REGEX_PATTERN_LEN,
            max_result_bytes: 64 * 1024 * 1024,
            max_share_bytes: 4 * 1024 * 1024,
            search_wall_clock_ms: 60_000,
            max_concurrent_search_runs: 5,
            sample_max_results: 1_000,
        }
    }
}
