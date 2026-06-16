use std::collections::BTreeSet;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use selene_core::{DbString, LabelSet, NodeId, PropertyMap, Value};

use super::TextIndex;
use crate::error::GraphResult;
use crate::graph::{SeleneGraph, TextIndexEntry};

type TextIndexMap = FxHashMap<(DbString, DbString), TextIndexEntry>;

pub(crate) fn apply_node_create(
    indexes: &mut TextIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            insert_commit(
                indexes,
                label.clone(),
                property.clone(),
                value,
                row,
                node_id,
            );
        }
    }
}

pub(crate) fn apply_node_delete(
    indexes: &mut TextIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            remove_commit(
                indexes,
                label.clone(),
                property.clone(),
                value,
                row,
                node_id,
            );
        }
    }
}

pub(crate) fn apply_node_update(
    indexes: &mut TextIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
    node_id: NodeId,
) {
    let candidates = candidate_keys(indexes, old_labels, old_props, new_labels, new_props);
    for (label, property) in candidates {
        match (
            indexable_text(old_labels, old_props, &label, &property),
            indexable_text(new_labels, new_props, &label, &property),
        ) {
            (Some(old_text), Some(new_text)) if old_text == new_text => {}
            (Some(_), Some(new_text)) => {
                insert_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    new_text,
                    row,
                    node_id,
                );
            }
            (Some(old_text), None) => {
                remove_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    old_text,
                    row,
                    node_id,
                );
            }
            (None, Some(new_text)) => {
                insert_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    new_text,
                    row,
                    node_id,
                );
            }
            (None, None) => {}
        }
    }
}

pub(crate) fn rebuild_text_indexes(graph: &mut SeleneGraph) -> GraphResult<()> {
    let registrations: Vec<((DbString, DbString), Option<DbString>)> = graph
        .text_index
        .iter()
        .map(|(key, entry)| (key.clone(), entry.name.clone()))
        .collect();
    graph.text_index.clear();
    for ((label, property), name) in registrations {
        let index = TextIndex::build(graph, label.clone(), property.clone())?;
        graph
            .text_index
            .insert((label, property), TextIndexEntry::new(index, name));
    }
    Ok(())
}

fn candidate_keys(
    indexes: &TextIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
) -> BTreeSet<(DbString, DbString)> {
    if indexes.is_empty() {
        return BTreeSet::new();
    }
    let mut labels: BTreeSet<DbString> = BTreeSet::new();
    labels.extend(old_labels.iter().cloned());
    labels.extend(new_labels.iter().cloned());

    let mut properties: BTreeSet<DbString> = BTreeSet::new();
    properties.extend(old_props.keys().cloned());
    properties.extend(new_props.keys().cloned());

    let mut candidates = BTreeSet::new();
    for label in &labels {
        for property in &properties {
            let key = (label.clone(), property.clone());
            if indexes.contains_key(&key) {
                candidates.insert(key);
            }
        }
    }
    candidates
}

fn indexable_text<'a>(
    labels: &LabelSet,
    props: &'a PropertyMap,
    label: &DbString,
    property: &DbString,
) -> Option<&'a str> {
    if !labels.contains(label) {
        return None;
    }
    match props.get(property) {
        Some(Value::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn insert_commit(
    indexes: &mut TextIndexMap,
    label: DbString,
    property: DbString,
    value: impl TextValue,
    row: u32,
    node_id: NodeId,
) {
    let Some(text) = value.text() else {
        return;
    };
    if let Some(entry) = indexes.get_mut(&(label, property)) {
        Arc::make_mut(&mut entry.index).insert_document(row, node_id, text);
    }
}

fn remove_commit(
    indexes: &mut TextIndexMap,
    label: DbString,
    property: DbString,
    value: impl TextValue,
    row: u32,
    node_id: NodeId,
) {
    if value.text().is_none() {
        return;
    }
    if let Some(entry) = indexes.get_mut(&(label, property)) {
        Arc::make_mut(&mut entry.index).remove_document(row, node_id);
    }
}

trait TextValue {
    fn text(&self) -> Option<&str>;
}

impl TextValue for &Value {
    fn text(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text.as_str()),
            _ => None,
        }
    }
}

impl TextValue for &str {
    fn text(&self) -> Option<&str> {
        Some(self)
    }
}
