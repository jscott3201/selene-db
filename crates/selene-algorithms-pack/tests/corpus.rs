//! Algorithms-pack corpus mirror tests.

use selene_algorithms_pack::ALGO_PROCEDURE_NAMES;
use selene_testing::AlgoPackCorpus;

#[test]
fn algo_pack_corpus_pagerank_entry_matches_golden() {
    let corpus = AlgoPackCorpus::b5_seed();

    insta::assert_snapshot!(corpus.render(), @r"
projection_build_all [Projection] CALL algo.projection_build('p', NULL, NULL, NULL)
projection_get [Projection] CALL algo.projection_get('p')
projection_drop [Projection] CALL algo.projection_drop('p')
projection_list [Projection] CALL algo.projection_list()
pagerank_defaults [Algorithm] CALL algo.pagerank('p', NULL, NULL, NULL)
betweenness_defaults [Algorithm] CALL algo.betweenness('p', NULL)
label_propagation_defaults [Algorithm] CALL algo.label_propagation('p', NULL)
louvain_defaults [Algorithm] CALL algo.louvain('p', NULL)
triangle_count [Algorithm] CALL algo.triangle_count('p')
wcc [Algorithm] CALL algo.wcc('p')
scc [Algorithm] CALL algo.scc('p')
wcc_count [Algorithm] CALL algo.wcc_count('p')
scc_count [Algorithm] CALL algo.scc_count('p')
topological_sort [Algorithm] CALL algo.topological_sort('p')
articulation_points [Algorithm] CALL algo.articulation_points('p')
bridges [Algorithm] CALL algo.bridges('p')
dijkstra [Algorithm] MATCH (source), (target) CALL algo.dijkstra('p', source, target) YIELD cost, path, length
sssp [Algorithm] MATCH (source) CALL algo.sssp('p', source) YIELD target_node, cost
apsp [Algorithm] CALL algo.apsp('p', 100)
");
}

#[test]
fn algo_pack_corpus_drift_detection_pins_registered_procedure_count() {
    let corpus = AlgoPackCorpus::b5_seed();

    assert_eq!(corpus.entries().len(), ALGO_PROCEDURE_NAMES.len());
}
