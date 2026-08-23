//! DDL Flagger walk.

use selene_profile::FeatureId;

use crate::{
    DdlStatement,
    ast::ddl::{DropBehavior, TypePropertyConstraint, TypePropertyDef},
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
            // BRIEF-152 / audit Item 10: DROP GRAPH ships as the IM_DROP_GRAPH
            // factory-reset extension (a supported selene-db vendor flag), NOT
            // GC04. CREATE GRAPH stays on GC04 (unsupported) so it remains
            // parse-rejected under D1 single-graph. IF EXISTS is informational
            // under D1 (the session graph always exists), so it carries no extra
            // flag — both DROP GRAPH and DROP GRAPH IF EXISTS flag IM_DROP_GRAPH.
            let _ = if_exists;
            record_feature(uses, FeatureId::IM_DROP_GRAPH, *span);
        }
        DdlStatement::CreateNodeType {
            key_label_set,
            extends,
            or_replace,
            if_not_exists,
            properties,
            span,
            ..
        } => {
            let _ = or_replace;
            type_ddl(*span, key_label_set.is_some(), uses);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
            if extends.is_some() {
                record_feature(uses, FeatureId::IM_EXTENDS, *span);
            }
            property_defs(properties, uses);
        }
        DdlStatement::CreateEdgeType {
            key_label_set,
            extends,
            or_replace,
            if_not_exists,
            properties,
            span,
            ..
        } => {
            let _ = or_replace;
            type_ddl(*span, key_label_set.is_some(), uses);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
            if extends.is_some() {
                record_feature(uses, FeatureId::IM_EXTENDS, *span);
            }
            property_defs(properties, uses);
        }
        DdlStatement::AlterNodeType {
            properties, span, ..
        } => {
            record_feature(uses, FeatureId::IM_ALTER_NODE_TYPE, *span);
            type_ddl(*span, false, uses);
            property_defs(properties, uses);
        }
        DdlStatement::AlterEdgeType {
            properties, span, ..
        } => {
            record_feature(uses, FeatureId::IM_ALTER_EDGE_TYPE, *span);
            type_ddl(*span, false, uses);
            property_defs(properties, uses);
        }
        DdlStatement::DropNodeType {
            if_exists,
            behavior,
            span,
            ..
        }
        | DdlStatement::DropEdgeType {
            if_exists,
            behavior,
            span,
            ..
        } => {
            type_ddl(*span, false, uses);
            if *if_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
            // GQL Flagger (clause 24.6): CASCADE is a selene-db impl-defined
            // addition, not ISO GQL, so it must flag on every use. RESTRICT and
            // the default carry only the existing type-DDL flags.
            if matches!(behavior, DropBehavior::Cascade) {
                record_feature(uses, FeatureId::IM_DROP_CASCADE, *span);
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
            type_ddl(*span, false, uses);
        }
        DdlStatement::ShowIndexes(_) | DdlStatement::ShowProcedures(_) => {}
    }
}

fn type_ddl(span: crate::SourceSpan, explicit_key_label_set: bool, uses: &mut Vec<FeatureUse>) {
    // ISO/IEC 39075:2024 §18.2/18.3: a closed-graph type-DDL statement uses an
    // explicit `<node/edge type name>` (the `:Name` after `NODE/EDGE TYPE`),
    // which is GG20 "Explicit element type names" under a closed graph type
    // (GG02).
    record_feature(uses, FeatureId::GG02, span);
    record_feature(uses, FeatureId::GG20, span);
    // GG21 "Explicit element type key label sets" flags only when the source
    // contains the explicit `<node/edge type key label set>` production (`[
    // <label set phrase> ] <implies>`, the `=>` marker). The bare `:Name` form
    // keeps the key label set *implied* per §18.2 SR5c — GG20-only, no GG21.
    if explicit_key_label_set {
        record_feature(uses, FeatureId::GG21, span);
    }
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
        | TypePropertyConstraint::Indexed { .. } => {}
    }
}
