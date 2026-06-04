#![allow(missing_docs)]
//! Criterion bench entrypoint for graph-augmented vector retrieval research.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
#[path = "vector_graph_retrieval/fixture.rs"]
mod fixture;

use criterion::{criterion_group, criterion_main};
use fixture::bench_graph_augmented_vector_retrieval;

criterion_group! {
    name = vector_graph_retrieval;
    config = common::criterion_config();
    targets = bench_graph_augmented_vector_retrieval
}
criterion_main!(vector_graph_retrieval);
