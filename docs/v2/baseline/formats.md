# Archive formats and registries

> Historical evidence for source `b8782bec34ff0b815b62711ac7e33cac09d8ea71` only.
> Not a 2.0 compatibility, signature, format-reader, alias, or migration contract.
> Benchmarks ran on an intentionally busy machine. They are non-green observations, not guards, comparisons, thresholds, or stable percentage baselines; issue #1137 / M08-PR06 owns future stable measurement.

Archive implementation identities only; no 2.0 reader or reopen test is authorized.

## Persistence

| Artifact | Magic | Version | Source | Source SHA-256 |
|---|---|---|---|---|
| WAL | `SLDB` | `3.0` | `crates/selene-persist/src/file_header.rs` | `7223977e79260631c402847a2519f5929fdd9b9e2a60b4e0af057db6b91391fb` |
| snapshot | `SLSN` | `1.5` | `crates/selene-persist/src/snapshot_file_header.rs` | `7508d352110d50a8f01882ae650a783ebccc16d28ba79027550f8b2884ff0aaa` |
| MANIFEST | `SLMF` | `1` | `crates/selene-persist/src/manifest.rs` | `a0c9f8329ffa5a82d34a391d46f64c8c8a3de43c415b40f64db5dd141e320e05` |
| audit | `SLAU` | `2` | `crates/selene-persist/src/audit.rs` | `97e9ea4dc0119fae2dec9feb68b617aab395aab0845ddf7359d75321fd1d8d3b` |

## Packages

| Package | Crate | Version | Published | Manifest SHA-256 |
|---|---|---|---:|---|
| `selene-db-core` | `selene_core` | `1.4.0` | yes | `601cde0c01e22ebd0fe9532cd7ac66ffd33cf795f3ddedb128199d97690e60aa` |
| `selene-db-graph` | `selene_graph` | `1.4.0` | yes | `5417fa0f87b297062a0e5b692d9e78884b6dfaf886351de5556c4dce819002d2` |
| `selene-db-persist` | `selene_persist` | `1.4.0` | yes | `733d1749ee13833647199fba0d5685695ffad2242f2378d89899e22520d06732` |
| `selene-db-algorithms` | `selene_algorithms` | `1.4.0` | yes | `c262b35e8b4b121f540ffa2fc22070874e93bfd548a9b28edc9acfd2072e9780` |
| `selene-db-gql` | `selene_gql` | `1.4.0` | yes | `806288705bce79757a68706588f8aad728fd9a9f9374ffc05749f69d77babae6` |
| `selene-db-testing` | `selene_testing` | `1.4.0` | no | `eec501b5115c2316546955f2811820387e650f74c037a618b1e856f103ef683b` |

## Procedure registries

### Builtin procedures (50)

Source `crates/selene-gql/src/runtime/builtins/catalog/specs.rs` (`471d688e794e82cb7a60093c3710cc7d1afec6588835298625a59b3dcd3c8873`).

`selene.health`, `selene.feature_status`, `selene.verify`, `selene.create_index`, `selene.drop_index`, `selene.compaction_stats`, `selene.vector_search_nodes`, `selene.vector_search_nodes_ann`, `selene.vector_index_stats`, `selene.text_index_stats`, `selene.property_index_stats`, `selene.json_contains_nodes`, `selene.json_path_exists_nodes`, `selene.json_path_contains_nodes`, `selene.json_path_value_nodes`, `selene.json_contains_candidate_nodes`, `selene.json_path_exists_candidate_nodes`, `selene.json_path_contains_candidate_nodes`, `selene.json_path_value_candidate_nodes`, `selene.rebuild_vector_indexes`, `selene.rebuild_recommended_vector_indexes`, `selene.compact`, `selene.create_vector_index`, `selene.drop_vector_index`, `selene.create_text_index`, `selene.drop_text_index`, `selene.vector_search_nodes_ann_batch`, `selene.vector_search_expanded_candidates_ann`, `selene.vector_search_candidate_state_expanded_ann`, `selene.vector_search_expanded_candidates_ann_batch`, `selene.vector_search_nodes_batch`, `selene.vector_score_nodes`, `selene.vector_score_nodes_batch`, `selene.vector_score_neighbors`, `selene.vector_score_neighbors_batch`, `selene.vector_score_candidate_state`, `selene.vector_score_candidate_state_nodes`, `selene.vector_score_candidate_state_expanded`, `selene.vector_score_candidate_state_expanded_batch`, `selene.vector_candidate_states`, `selene.reachable_nodes`, `selene.vector_score_expanded_candidates`, `selene.vector_score_expanded_candidates_batch`, `selene.text_search_nodes`, `selene.text_score_nodes`, `selene.text_score_nodes_batch`, `selene.text_score_candidate_state`, `selene.text_score_candidate_state_nodes`, `selene.text_score_candidate_state_expanded_batch`, `selene.reciprocal_rank_fusion`

### Algorithm procedures (19)

Source `crates/selene-gql/src/runtime/native_algorithms/mod.rs` (`78e99906ddf5f07e217f7e5149643acd2b9942787914531a5e6777718d82d22a`).

`algo.projection_build`, `algo.projection_get`, `algo.projection_drop`, `algo.projection_list`, `algo.pagerank`, `algo.betweenness`, `algo.label_propagation`, `algo.louvain`, `algo.triangle_count`, `algo.wcc`, `algo.scc`, `algo.wcc_count`, `algo.scc_count`, `algo.topological_sort`, `algo.articulation_points`, `algo.bridges`, `algo.dijkstra`, `algo.sssp`, `algo.apsp`

## Feature register

`crates/selene-core/src/feature_register.rs` (`70de7ec160b29386954406844aca2a7b991b280ac3973f134530390e50b37bff`): 176 referenced, 143 supported, 32 non-support rationales.

`build/regen_feature_docs.sh` is a **placeholder** (`70357f61114f94856cc6c64a277423e7a2e8c10708279fac2c354b5c7fe1118d`), not generated authority.
