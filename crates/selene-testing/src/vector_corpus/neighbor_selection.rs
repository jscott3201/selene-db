//! HNSW neighbor-selection corpus entries.

use super::coverage::{
    NeighborSelectionFlavor, QuantMethodMirror, VectorErrorKindMirror, VectorMagicMirror,
    VectorMetricMirror, VectorOpMirror, VectorSurface,
};
use super::fixtures::{VectorCorpusGraph, VectorQuantizationSpec};
use super::{VectorCorpusCategory, VectorCorpusEntry, cfg_neighbor_hnsw, search};

const SEARCH_SURFACES: &[VectorSurface] = &[
    VectorSurface::HnswBuild,
    VectorSurface::HnswSearchUnfiltered,
];
const L2_METRIC: &[VectorMetricMirror] = &[VectorMetricMirror::L2];
const INSERT_OP: &[VectorOpMirror] = &[VectorOpMirror::Insert];
const GRAPH_MAGICS: &[VectorMagicMirror] = &[
    VectorMagicMirror::Vecb,
    VectorMagicMirror::Vgrp,
    VectorMagicMirror::Vvec,
];
const NO_ERRORS: &[VectorErrorKindMirror] = &[];
const NO_QUANT: &[QuantMethodMirror] = &[];

pub(crate) fn search_entry(
    slug: &'static str,
    description: &'static str,
    flavor: NeighborSelectionFlavor,
) -> VectorCorpusEntry {
    VectorCorpusEntry {
        slug,
        description,
        graph: VectorCorpusGraph::DiverseClusterL2_64,
        config: cfg_neighbor_hnsw(
            8,
            VectorMetricMirror::L2,
            VectorQuantizationSpec::DISABLED,
            flavor,
            8,
            64,
            50,
        ),
        invocation: search(&[0.0; 8], 8, Some(50), None),
        category: VectorCorpusCategory::Search,
        covered_surfaces: SEARCH_SURFACES,
        covered_metrics: L2_METRIC,
        covered_ops: INSERT_OP,
        covered_errors: NO_ERRORS,
        covered_magics: GRAPH_MAGICS,
        covered_quant_methods: NO_QUANT,
    }
}
