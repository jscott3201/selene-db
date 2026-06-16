use super::*;

#[test]
fn vector_index_tracks_create_update_and_delete_membership() {
    let shared = SharedGraph::new(GraphId::new(8101));
    let label = db_string("vector.index.doc");
    let property = db_string("embedding");
    let other = db_string("vector.index.other");
    let (doc_a, doc_b) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let doc_a = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[1.0, 0.0]))]),
            )
            .unwrap();
        let doc_b = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(other, Value::Int(9))]),
            )
            .unwrap();
        txn.commit().unwrap();
        (doc_a, doc_b)
    };

    shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 2)
        .unwrap();
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc_b,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), vector(&[0.0, 1.0]))], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![0, 1]
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(doc_a).unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn create_vector_index_rejects_existing_wrong_kind() {
    let shared = SharedGraph::new(GraphId::new(8102));
    let label = db_string("vector.index.kind");
    let property = db_string("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), Value::String(db_string("not-vector")))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 3,
            observed,
        } if err_label == label && err_property == property && observed == "String"
    ));
}

#[test]
fn create_vector_index_rejects_existing_dimension_mismatch() {
    let shared = SharedGraph::new(GraphId::new(8103));
    let label = db_string("vector.index.dimension");
    let property = db_string("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[1.0, 2.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 3,
            observed,
        } if err_label == label && err_property == property && observed == "VECTOR<2>"
    ));
}

#[test]
fn create_hnsw_cosine_index_rejects_existing_zero_norm_vector() {
    let shared = SharedGraph::new(GraphId::new(8106));
    let label = db_string("vector.index.cosine.zero");
    let property = db_string("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[0.0, 0.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswCosine,
            2,
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 2,
            observed,
        } if err_label == label
            && err_property == property
            && observed.contains("zero-norm vector")
    ));
}

#[test]
fn indexed_vector_property_rejects_later_dimension_drift() {
    let shared = SharedGraph::new(GraphId::new(8104));
    let label = db_string("vector.index.strict");
    let property = db_string("embedding");
    let doc = {
        let mut txn = shared.begin_write();
        let doc = txn
            .mutator()
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        doc
    };
    shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 2)
        .unwrap();

    let err = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), vector(&[1.0, 2.0, 3.0]))], []).unwrap(),
            )
            .unwrap_err()
    };

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 2,
            observed,
        } if err_label == label && err_property == property && observed == "VECTOR<3>"
    ));
}
