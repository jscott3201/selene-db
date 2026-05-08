//! Mutation Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    MutationPipeline, MutationStatement, MutationTerminator,
    ast::mutation::{InsertStatement, RemoveItem, SetItem},
    error::ParserError,
};

use super::{check_feature, expr, query};

pub(crate) fn pipeline(pipeline: &MutationPipeline) -> Result<(), ParserError> {
    check_feature(FeatureId::GD01, pipeline.span)?;
    for statement in &pipeline.statements {
        mutation_statement(statement)?;
    }
    if let Some(terminator) = &pipeline.terminator {
        match terminator {
            MutationTerminator::Return(clause) => query::return_clause(clause)?,
            MutationTerminator::Finish(_) => {}
        }
    }
    Ok(())
}

fn mutation_statement(statement: &MutationStatement) -> Result<(), ParserError> {
    match statement {
        MutationStatement::Match(value) => query::match_clause(value),
        MutationStatement::Filter(value) => expr::value(value),
        MutationStatement::Insert(value) => insert(value),
        MutationStatement::Set(values) => set_items(values),
        MutationStatement::Remove(values) => remove_items(values),
        MutationStatement::Delete(_) => Ok(()),
    }
}

fn insert(statement: &InsertStatement) -> Result<(), ParserError> {
    for pattern in &statement.patterns {
        query::graph_pattern(pattern)?;
    }
    Ok(())
}

fn set_items(items: &[SetItem]) -> Result<(), ParserError> {
    for item in items {
        match item {
            SetItem::Property { value, .. } => expr::value(value)?,
            SetItem::PropertyMerge { properties, .. } => {
                for (_, value) in properties {
                    expr::value(value)?;
                }
            }
            SetItem::Label { .. } => {}
        }
    }
    Ok(())
}

fn remove_items(items: &[RemoveItem]) -> Result<(), ParserError> {
    for item in items {
        match item {
            RemoveItem::Property { .. } | RemoveItem::Label { .. } => {}
        }
    }
    Ok(())
}
