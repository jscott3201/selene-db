//! DDL Flagger walk.

use selene_core::feature_register::FeatureId;

use crate::{
    DdlStatement,
    ast::ddl::{TypePropertyConstraint, TypePropertyDef},
};

use super::{FeatureUse, expr, record_feature};

pub(crate) fn statement(statement: &DdlStatement, uses: &mut Vec<FeatureUse>) {
    match statement {
        DdlStatement::CreateGraph {
            or_replace,
            if_not_exists,
            span,
            ..
        } => {
            let _ = or_replace;
            record_feature(uses, FeatureId::GG01, *span);
            record_feature(uses, FeatureId::GC04, *span);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC05, *span);
            }
        }
        DdlStatement::DropGraph {
            if_exists, span, ..
        } => {
            record_feature(uses, FeatureId::GC04, *span);
            if *if_exists {
                record_feature(uses, FeatureId::GC05, *span);
            }
        }
        DdlStatement::CreateNodeType {
            extends,
            or_replace,
            if_not_exists,
            properties,
            span,
            ..
        } => {
            let _ = or_replace;
            type_ddl(*span, uses);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
            if extends.is_some() {
                record_feature(uses, FeatureId::IM_EXTENDS, *span);
            }
            property_defs(properties, uses);
        }
        DdlStatement::CreateEdgeType {
            extends,
            or_replace,
            if_not_exists,
            properties,
            span,
            ..
        } => {
            let _ = or_replace;
            type_ddl(*span, uses);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
            if extends.is_some() {
                record_feature(uses, FeatureId::IM_EXTENDS, *span);
            }
            property_defs(properties, uses);
        }
        DdlStatement::DropNodeType {
            if_exists, span, ..
        }
        | DdlStatement::DropEdgeType {
            if_exists, span, ..
        } => {
            type_ddl(*span, uses);
            if *if_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
        }
        DdlStatement::CreateIndex { span, .. } | DdlStatement::DropIndex { span, .. } => {
            record_feature(uses, FeatureId::IM_INDEX_DDL, *span);
        }
        // GQL Flagger (clause 24.6): TRUNCATE is a selene-db impl-defined
        // addition, not ISO GQL, so it must flag on every use.
        DdlStatement::TruncateNodeType { span, .. }
        | DdlStatement::TruncateEdgeType { span, .. } => {
            record_feature(uses, FeatureId::IM_TRUNCATE, *span);
        }
        DdlStatement::ShowNodeTypes(span) | DdlStatement::ShowEdgeTypes(span) => {
            type_ddl(*span, uses);
        }
        DdlStatement::ShowIndexes(_) | DdlStatement::ShowProcedures(_) => {}
    }
}

fn type_ddl(span: crate::SourceSpan, uses: &mut Vec<FeatureUse>) {
    record_feature(uses, FeatureId::GG02, span);
    record_feature(uses, FeatureId::GG20, span);
    record_feature(uses, FeatureId::GG21, span);
}

fn property_defs(properties: &[TypePropertyDef], uses: &mut Vec<FeatureUse>) {
    for property in properties {
        expr::gql_type(&property.gql_type, property.span, uses);
        for constraint in &property.constraints {
            property_constraint(constraint, uses);
        }
    }
}

fn property_constraint(constraint: &TypePropertyConstraint, uses: &mut Vec<FeatureUse>) {
    match constraint {
        TypePropertyConstraint::Default(value, _) => expr::value(value, uses),
        TypePropertyConstraint::NotNull(_)
        | TypePropertyConstraint::Immutable(_)
        | TypePropertyConstraint::Unique(_)
        | TypePropertyConstraint::Indexed { .. }
        | TypePropertyConstraint::Searchable(_)
        | TypePropertyConstraint::Dictionary(_)
        | TypePropertyConstraint::Fill(_, _)
        | TypePropertyConstraint::Interval(_, _)
        | TypePropertyConstraint::Encoding(_, _) => {}
    }
}
