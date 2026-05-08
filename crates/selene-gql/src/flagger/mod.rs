//! ISO optional-feature gate over parsed AST.

mod call;
mod ddl;
mod expr;
mod mutation;
mod query;

use selene_core::feature_register::{FeatureId, is_supported, name_of, non_supported_rationale};

use crate::{SourceSpan, Statement, error::ParserError};

pub(crate) fn flag(statement: &Statement) -> Result<(), ParserError> {
    query::statement(statement)
}

fn check_feature(id: FeatureId, span: SourceSpan) -> Result<(), ParserError> {
    if is_supported(id) {
        return Ok(());
    }
    Err(ParserError::UnsupportedFeature {
        feature_id: id,
        display_name: name_of(id).unwrap_or("unnamed feature"),
        span,
        hint: non_supported_rationale(id)
            .unwrap_or("feature is outside the selene-db v1.0 claim list"),
    })
}
