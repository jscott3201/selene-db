//! Vector-pack mirror corpus tests.

use selene_testing::VectorPackCorpus;
use selene_vector_pack::VECTOR_PROCEDURE_NAMES;

#[test]
fn vector_pack_corpus_renders_in_deterministic_order_and_covers_registry() {
    let corpus = VectorPackCorpus::b3_seed();
    let observed = corpus
        .entries()
        .iter()
        .map(|entry| entry.invocation.procedure_name())
        .collect::<Vec<_>>();

    assert_eq!(observed, VECTOR_PROCEDURE_NAMES.to_vec());
    insta::assert_snapshot!(corpus.render(), @r"
search_default [Search] CALL vector.search('default', [1.000000, 0.000000, 0.000000, 0.000000], 10, NULL, NULL)
upsert_default [Upsert] CALL vector.upsert('default', 42, [1.000000, 0.000000, 0.000000, 0.000000])
delete_default [Delete] CALL vector.delete('default', 42)
bulk_upsert_default [BulkUpsert] CALL vector.bulk_upsert('default', [42, 43], [[1.000000, 0.000000, 0.000000, 0.000000], [0.000000, 1.000000, 0.000000, 0.000000]])
bulk_delete_default [BulkDelete] CALL vector.bulk_delete('default', [42, 43])
ivf_bulk_upsert_default [IvfBulkUpsert] CALL vector.ivf_bulk_upsert('default', [42, 43], [[1.000000, 0.000000, 0.000000, 0.000000], [0.000000, 1.000000, 0.000000, 0.000000]])
ivf_bulk_delete_default [IvfBulkDelete] CALL vector.ivf_bulk_delete('default', [42, 43])
");
}
