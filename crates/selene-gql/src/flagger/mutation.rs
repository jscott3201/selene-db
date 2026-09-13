//! Mutation Flagger walk.

use selene_profile::FeatureId;

use crate::{
    MutationPipeline, MutationStatement, MutationTerminator,
    ast::mutation::{InsertStatement, RemoveItem, SetItem},
};

use super::{FeatureUse, expr, query, record_feature};

pub(crate) fn pipeline(pipeline: &MutationPipeline, uses: &mut Vec<FeatureUse>) {
    record_feature(uses, FeatureId::GD01, pipeline.span);
    for statement in &pipeline.statements {
        mutation_statement(statement, uses);
    }
    if let Some(terminator) = &pipeline.terminator {
        match terminator {
            MutationTerminator::Return(clause) => query::return_clause(clause, uses),
            MutationTerminator::Finish(_) => {}
        }
    }
}

fn mutation_statement(statement: &MutationStatement, uses: &mut Vec<FeatureUse>) {
    match statement {
        MutationStatement::Match(value) => query::match_clause(value, uses),
        MutationStatement::Filter(value) => expr::value(value, uses),
        MutationStatement::Insert(value) => insert(value, uses),
        MutationStatement::Set(values) => set_items(values, uses),
        MutationStatement::Remove(values) => remove_items(values, uses),
        MutationStatement::Delete(_) => {}
    }
}

fn insert(statement: &InsertStatement, uses: &mut Vec<FeatureUse>) {
    for pattern in &statement.patterns {
        query::graph_pattern(pattern, uses);
    }
}

fn set_items(items: &[SetItem], uses: &mut Vec<FeatureUse>) {
    for item in items {
        match item {
            SetItem::Property { value, .. } => expr::value(value, uses),
            SetItem::PropertyMerge { properties, .. } => {
                for (_, value) in properties {
                    expr::value(value, uses);
                }
            }
            SetItem::Label { .. } => {}
        }
    }
}

fn remove_items(items: &[RemoveItem], _uses: &mut Vec<FeatureUse>) {
    for item in items {
        match item {
            RemoveItem::Property { .. } | RemoveItem::Label { .. } => {}
        }
    }
}
