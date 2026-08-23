//! ISO optional-feature compatibility gate over parsed AST.

mod call;
mod ddl;
mod expr;
mod mutation;
mod query;

use selene_core::feature_register::{
    FeatureId, is_flagger_accepted, name_of, non_supported_rationale,
};

use crate::{SourceSpan, Statement, error::ParserError};

/// One optional feature observed while walking an AST.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureUse {
    /// ISO feature identifier.
    pub feature_id: FeatureId,
    /// Source span that exercises the feature.
    pub span: SourceSpan,
}

/// Return every optional feature surface reached by `statement`.
#[must_use]
pub fn feature_walk(statement: &Statement) -> Vec<FeatureUse> {
    let mut uses = Vec::new();
    query::statement(statement, &mut uses);
    uses
}

pub(crate) fn flag(statement: &Statement) -> Result<(), ParserError> {
    reject_unimplemented(statement)?;
    for feature in feature_walk(statement) {
        check_feature(feature.feature_id, feature.span)?;
    }
    Ok(())
}

pub(super) fn record_feature(uses: &mut Vec<FeatureUse>, id: FeatureId, span: SourceSpan) {
    uses.push(FeatureUse {
        feature_id: id,
        span,
    });
}

fn check_feature(id: FeatureId, span: SourceSpan) -> Result<(), ParserError> {
    if is_flagger_accepted(id) {
        return Ok(());
    }
    Err(ParserError::UnsupportedFeature {
        feature_id: id,
        display_name: name_of(id).unwrap_or("unnamed feature"),
        span,
        hint: non_supported_rationale(id)
            .unwrap_or("feature is outside the parser compatibility set"),
    })
}

fn reject_unimplemented(statement: &Statement) -> Result<(), ParserError> {
    if let Statement::Ddl(
        crate::DdlStatement::CreateGraph {
            or_replace: true,
            span,
            ..
        }
        | crate::DdlStatement::CreateNodeType {
            or_replace: true,
            span,
            ..
        }
        | crate::DdlStatement::CreateEdgeType {
            or_replace: true,
            span,
            ..
        },
    ) = statement
    {
        return Err(ParserError::not_implemented(
            "OR REPLACE is not part of ISO/IEC 39075:2024 catalog DDL",
            *span,
            Some("OR REPLACE has no ISO feature ID; drop the modifier or DROP+CREATE explicitly"),
        ));
    }
    Ok(())
}
