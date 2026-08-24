//! DDL Flagger walk.

use selene_profile::FeatureId;

use crate::{
    DdlStatement,
    ast::ddl::{DropBehavior, TypePropertyConstraint, TypePropertyDef},
};

use super::{FeatureUse, expr, record_feature};

pub(crate) fn statement(statement: &DdlStatement, uses: &mut Vec<FeatureUse>) {
    match statement {
        // ISO/IEC 39075:2024 section 12.2/12.3 CR1-CR2: schema statements are
        // GC01; the conditional modifier adds GC02.
        DdlStatement::CreateSchema {
            if_not_exists: conditional,
            span,
            ..
        }
        | DdlStatement::DropSchema {
            if_exists: conditional,
            span,
            ..
        } => {
            record_feature(uses, FeatureId::GC01, *span);
            if *conditional {
                record_feature(uses, FeatureId::GC02, *span);
            }
        }
        // Section 12.4: CREATE GRAPH is GC04 and the conditional modifier adds
        // GC05. The type clause records GG01 for an open graph or GG02 for a
        // named closed graph. OR REPLACE has no feature of its own.
        DdlStatement::CreateGraph {
            if_not_exists,
            graph_type,
            span,
            ..
        } => {
            record_feature(uses, FeatureId::GC04, *span);
            record_feature(
                uses,
                if graph_type.is_some() {
                    FeatureId::GG02
                } else {
                    FeatureId::GG01
                },
                *span,
            );
            if *if_not_exists {
                record_feature(uses, FeatureId::GC05, *span);
            }
        }
        // Section 12.5 CR1-CR2: DROP GRAPH is GC04 (+GC05 with IF EXISTS).
        // IM_DROP_GRAPH is still stamped as well: the Flagger is static and
        // cannot know whether the reference resolves to the protected bootstrap
        // graph, which the compatibility session factory-resets through the
        // lower engine instead of dropping. Until M02-PR05 deletes that bridge,
        // every DROP GRAPH may take the implementation-defined processing
        // alternative, which is exactly what section 24.6 asks a Flagger to
        // report. The stamp is removed together with the bridge.
        DdlStatement::DropGraph {
            if_exists, span, ..
        } => {
            record_feature(uses, FeatureId::GC04, *span);
            if *if_exists {
                record_feature(uses, FeatureId::GC05, *span);
            }
            record_feature(uses, FeatureId::IM_DROP_GRAPH, *span);
        }
        DdlStatement::CreateGraphType {
            if_not_exists,
            span,
            ..
        } => {
            record_feature(uses, FeatureId::GG02, *span);
            record_feature(uses, FeatureId::GG20, *span);
            if *if_not_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
        }
        DdlStatement::DropGraphType {
            if_exists, span, ..
        } => {
            record_feature(uses, FeatureId::GG02, *span);
            if *if_exists {
                record_feature(uses, FeatureId::GC03, *span);
            }
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
