//! DDL Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    DdlStatement,
    ast::ddl::{TypePropertyConstraint, TypePropertyDef},
    error::ParserError,
};

use super::{check_feature, expr};

pub(crate) fn statement(statement: &DdlStatement) -> Result<(), ParserError> {
    match statement {
        DdlStatement::CreateGraph {
            if_not_exists,
            span,
            ..
        } => {
            check_feature(FeatureId::GC04, *span)?;
            if *if_not_exists {
                check_feature(FeatureId::GC03, *span)?;
            }
            Ok(())
        }
        DdlStatement::DropGraph {
            if_exists, span, ..
        } => {
            check_feature(FeatureId::GC04, *span)?;
            if *if_exists {
                check_feature(FeatureId::GC03, *span)?;
            }
            Ok(())
        }
        DdlStatement::CreateNodeType {
            if_not_exists,
            properties,
            span,
            ..
        }
        | DdlStatement::CreateEdgeType {
            if_not_exists,
            properties,
            span,
            ..
        } => {
            type_ddl(*span)?;
            if *if_not_exists {
                check_feature(FeatureId::GC03, *span)?;
            }
            property_defs(properties)
        }
        DdlStatement::DropNodeType {
            if_exists, span, ..
        }
        | DdlStatement::DropEdgeType {
            if_exists, span, ..
        } => {
            type_ddl(*span)?;
            if *if_exists {
                check_feature(FeatureId::GC03, *span)?;
            }
            Ok(())
        }
        DdlStatement::ShowNodeTypes(span) | DdlStatement::ShowEdgeTypes(span) => type_ddl(*span),
    }
}

fn type_ddl(span: crate::SourceSpan) -> Result<(), ParserError> {
    check_feature(FeatureId::GG02, span)?;
    check_feature(FeatureId::GG20, span)?;
    check_feature(FeatureId::GG21, span)
}

fn property_defs(properties: &[TypePropertyDef]) -> Result<(), ParserError> {
    for property in properties {
        for constraint in &property.constraints {
            property_constraint(constraint)?;
        }
    }
    Ok(())
}

fn property_constraint(constraint: &TypePropertyConstraint) -> Result<(), ParserError> {
    match constraint {
        TypePropertyConstraint::Default(value, _) => expr::value(value),
        TypePropertyConstraint::NotNull(_)
        | TypePropertyConstraint::Immutable(_)
        | TypePropertyConstraint::Unique(_)
        | TypePropertyConstraint::Indexed(_)
        | TypePropertyConstraint::Searchable(_)
        | TypePropertyConstraint::Dictionary(_)
        | TypePropertyConstraint::Fill(_, _)
        | TypePropertyConstraint::Interval(_, _)
        | TypePropertyConstraint::Encoding(_, _) => Ok(()),
    }
}
