use selene_core::{
    CancellationChecker, GraphId, LabelSet, PropertyMap, Value, VectorValue, db_string,
};

use super::*;
use crate::vector_index::VectorIndexSearchHit;

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn ann_row_hits_to_node_hits_filters_dead_rows_and_orders_ties_by_node_id() {
    let shared = SharedGraph::new(GraphId::new(991));
    let doc = db_string("vector.ann.convert.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let (first, second, third) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0]))),
            )
            .unwrap();
        let second = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[2.0]))),
            )
            .unwrap();
        let third = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[3.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
        (first, second, third)
    };
    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(second).unwrap();
        txn.commit().unwrap();
    }

    let graph = shared.read();
    let hits = ann_row_hits_to_node_hits(
        graph.as_ref(),
        &doc,
        vec![
            VectorIndexSearchHit {
                row: 2,
                distance: 1.0,
            },
            VectorIndexSearchHit {
                row: 1,
                distance: 0.0,
            },
            VectorIndexSearchHit {
                row: 0,
                distance: 1.0,
            },
        ],
        &CancellationChecker::disabled(),
    )
    .unwrap();

    assert_eq!(
        hits,
        vec![
            VectorNodeSearchHit {
                node_id: first,
                distance: 1.0,
            },
            VectorNodeSearchHit {
                node_id: third,
                distance: 1.0,
            },
        ]
    );
}
