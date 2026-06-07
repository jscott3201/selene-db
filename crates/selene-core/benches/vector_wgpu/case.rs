use criterion::BenchmarkId;

pub(crate) const TOP_K: usize = 10;
pub(crate) const HOT_SHARD_REUSE_BATCHES: usize = 8;
const CANDIDATE_BLOCK: usize = 256;

const DEFAULT_CASES: &[Case] = &[
    Case {
        queries: 8,
        candidates: 4096,
        dimension: 1024,
    },
    Case {
        queries: 16,
        candidates: 4096,
        dimension: 1024,
    },
    Case {
        queries: 8,
        candidates: 4096,
        dimension: 2560,
    },
    Case {
        queries: 8,
        candidates: 10_000,
        dimension: 1024,
    },
    Case {
        queries: 16,
        candidates: 10_000,
        dimension: 1024,
    },
];

const STRESS_CASES: &[Case] = &[Case {
    queries: 8,
    candidates: 100_000,
    dimension: 1024,
}];

pub(crate) fn cases() -> Vec<Case> {
    let mut cases = DEFAULT_CASES.to_vec();
    if std::env::var_os("SELENE_WGPU_STRESS_CASES").is_some() {
        cases.extend_from_slice(STRESS_CASES);
    }
    cases
}

#[derive(Clone, Copy)]
pub(crate) struct Case {
    pub(crate) queries: usize,
    pub(crate) candidates: usize,
    pub(crate) dimension: usize,
}

impl Case {
    pub(crate) fn id(self, name: &str) -> BenchmarkId {
        BenchmarkId::new(
            name,
            format!("q{}x{}x{}", self.queries, self.candidates, self.dimension),
        )
    }

    pub(crate) const fn score_count(self) -> usize {
        self.queries * self.candidates
    }

    pub(crate) const fn hot_shard_score_count(self) -> usize {
        self.score_count() * HOT_SHARD_REUSE_BATCHES
    }

    pub(crate) const fn has_hot_shard_reuse_row(self) -> bool {
        self.queries == 16
            && self.dimension == 1024
            && (self.candidates == 4096 || self.candidates == 10_000)
    }

    pub(crate) const fn partial_count(self) -> usize {
        self.queries * self.block_count() * TOP_K
    }

    pub(crate) const fn output_bytes(self) -> u64 {
        (self.score_count() * size_of::<f32>()) as u64
    }

    pub(crate) const fn block_count(self) -> usize {
        self.candidates.div_ceil(CANDIDATE_BLOCK)
    }

    pub(crate) const fn partial_f32_bytes(self) -> u64 {
        (self.partial_count() * size_of::<f32>()) as u64
    }

    pub(crate) const fn partial_u32_bytes(self) -> u64 {
        (self.partial_count() * size_of::<u32>()) as u64
    }

    pub(crate) const fn partial_hit_bytes(self) -> u64 {
        (self.partial_count() * (size_of::<f32>() + size_of::<u32>())) as u64
    }

    pub(crate) fn largest_storage_bytes(self) -> u64 {
        [
            (self.queries * self.dimension * size_of::<f32>()) as u64,
            (self.candidates * self.dimension * size_of::<f32>()) as u64,
            ((self.queries + self.candidates) * size_of::<f32>()) as u64,
            self.output_bytes(),
            self.partial_f32_bytes(),
            self.partial_u32_bytes(),
            self.partial_hit_bytes(),
        ]
        .into_iter()
        .max()
        .expect("case storage byte candidates are non-empty")
    }
}
