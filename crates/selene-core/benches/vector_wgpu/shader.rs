pub(crate) const SCORE_SHADER: &str = r#"
struct Params {
    queries: u32,
    candidates: u32,
    dimension: u32,
    _padding: u32,
};

@group(0) @binding(0) var<storage, read> queries: array<f32>;
@group(0) @binding(1) var<storage, read> candidates: array<f32>;
@group(0) @binding(2) var<storage, read> norms: array<f32>;
@group(0) @binding(3) var<storage, read_write> distances: array<f32>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let score_index = id.x;
    let total = params.queries * params.candidates;
    if (score_index >= total) {
        return;
    }

    let query_index = score_index / params.candidates;
    let candidate_index = score_index % params.candidates;
    let query_offset = query_index * params.dimension;
    let candidate_offset = candidate_index * params.dimension;

    var dot = 0.0;
    for (var dim = 0u; dim < params.dimension; dim = dim + 1u) {
        dot = dot + queries[query_offset + dim] * candidates[candidate_offset + dim];
    }

    let denom = sqrt(norms[query_index]) * sqrt(norms[params.queries + candidate_index]);
    let similarity = clamp(dot / denom, -1.0, 1.0);
    var distance = 1.0 - similarity;
    if (distance == 0.0) {
        distance = 0.0;
    }
    distances[score_index] = distance;
}
"#;

pub(crate) const BLOCK_TOP_K_SHADER: &str = r#"
struct Params {
    queries: u32,
    candidates: u32,
    dimension: u32,
    _padding: u32,
};

@group(0) @binding(0) var<storage, read> distances: array<f32>;
@group(0) @binding(1) var<storage, read_write> partial_distances: array<f32>;
@group(0) @binding(2) var<storage, read_write> partial_indices: array<u32>;
@group(0) @binding(3) var<uniform> params: Params;

const TOP_K: u32 = 10u;
const CANDIDATE_BLOCK: u32 = 256u;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let block_index = id.x;
    let query_index = id.y;
    let block_count = (params.candidates + CANDIDATE_BLOCK - 1u) / CANDIDATE_BLOCK;
    if (query_index >= params.queries || block_index >= block_count) {
        return;
    }

    var best_distances: array<f32, 10>;
    var best_indices: array<u32, 10>;
    for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
        best_distances[slot] = 100000000000000000000.0;
        best_indices[slot] = 4294967295u;
    }

    let start = block_index * CANDIDATE_BLOCK;
    let end = min(start + CANDIDATE_BLOCK, params.candidates);
    for (var candidate = start; candidate < end; candidate = candidate + 1u) {
        let distance = distances[query_index * params.candidates + candidate];
        var insert = TOP_K;
        for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
            if (distance < best_distances[slot] ||
                (distance == best_distances[slot] && candidate < best_indices[slot])) {
                insert = slot;
                break;
            }
        }
        if (insert < TOP_K) {
            var slot = TOP_K - 1u;
            loop {
                if (slot <= insert) {
                    break;
                }
                best_distances[slot] = best_distances[slot - 1u];
                best_indices[slot] = best_indices[slot - 1u];
                slot = slot - 1u;
            }
            best_distances[insert] = distance;
            best_indices[insert] = candidate;
        }
    }

    let out_base = (query_index * block_count + block_index) * TOP_K;
    for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
        partial_distances[out_base + slot] = best_distances[slot];
        partial_indices[out_base + slot] = best_indices[slot];
    }
}
"#;

pub(crate) const FUSED_BLOCK_TOP_K_SHADER: &str = r#"
struct Params {
    queries: u32,
    candidates: u32,
    dimension: u32,
    _padding: u32,
};

@group(0) @binding(0) var<storage, read> queries: array<f32>;
@group(0) @binding(1) var<storage, read> candidates: array<f32>;
@group(0) @binding(2) var<storage, read> norms: array<f32>;
struct PartialHit {
    distance: f32,
    index: u32,
};
@group(0) @binding(3) var<storage, read_write> partial_hits: array<PartialHit>;
@group(0) @binding(4) var<uniform> params: Params;

const TOP_K: u32 = 10u;
const CANDIDATE_BLOCK: u32 = 256u;
const EMPTY_INDEX: u32 = 4294967295u;
const DISTANCE_SENTINEL: f32 = 100000000000000000000.0;

var<workgroup> block_distances: array<f32, 256>;
var<workgroup> block_indices: array<u32, 256>;

@compute @workgroup_size(256)
fn main(
    @builtin(local_invocation_id) local_id: vec3<u32>,
    @builtin(workgroup_id) group_id: vec3<u32>,
) {
    let lane = local_id.x;
    let block_index = group_id.x;
    let query_index = group_id.y;
    let block_count = (params.candidates + CANDIDATE_BLOCK - 1u) / CANDIDATE_BLOCK;
    if (query_index >= params.queries || block_index >= block_count) {
        return;
    }

    let candidate_index = block_index * CANDIDATE_BLOCK + lane;
    var distance = DISTANCE_SENTINEL;
    var stored_index = EMPTY_INDEX;
    if (candidate_index < params.candidates) {
        let query_offset = query_index * params.dimension;
        let candidate_offset = candidate_index * params.dimension;

        var dot = 0.0;
        for (var dim = 0u; dim < params.dimension; dim = dim + 1u) {
            dot = dot + queries[query_offset + dim] * candidates[candidate_offset + dim];
        }

        let denom = sqrt(norms[query_index]) * sqrt(norms[params.queries + candidate_index]);
        let similarity = clamp(dot / denom, -1.0, 1.0);
        distance = 1.0 - similarity;
        if (distance == 0.0) {
            distance = 0.0;
        }
        stored_index = candidate_index;
    }

    block_distances[lane] = distance;
    block_indices[lane] = stored_index;
    workgroupBarrier();

    if (lane == 0u) {
        var best_distances: array<f32, 10>;
        var best_indices: array<u32, 10>;
        for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
            best_distances[slot] = DISTANCE_SENTINEL;
            best_indices[slot] = EMPTY_INDEX;
        }

        for (var offset = 0u; offset < CANDIDATE_BLOCK; offset = offset + 1u) {
            let candidate = block_indices[offset];
            if (candidate != EMPTY_INDEX) {
                let candidate_distance = block_distances[offset];
                var insert = TOP_K;
                for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
                    if (candidate_distance < best_distances[slot] ||
                        (candidate_distance == best_distances[slot] &&
                         candidate < best_indices[slot])) {
                        insert = slot;
                        break;
                    }
                }
                if (insert < TOP_K) {
                    var slot = TOP_K - 1u;
                    loop {
                        if (slot <= insert) {
                            break;
                        }
                        best_distances[slot] = best_distances[slot - 1u];
                        best_indices[slot] = best_indices[slot - 1u];
                        slot = slot - 1u;
                    }
                    best_distances[insert] = candidate_distance;
                    best_indices[insert] = candidate;
                }
            }
        }

        let out_base = (query_index * block_count + block_index) * TOP_K;
        for (var slot = 0u; slot < TOP_K; slot = slot + 1u) {
            partial_hits[out_base + slot] = PartialHit(best_distances[slot], best_indices[slot]);
        }
    }
}
"#;
