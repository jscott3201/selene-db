//! Generated-profile capability gate over parsed AST.

mod call;
mod ddl;
mod expr;
mod mutation;
mod query;

use selene_profile::{FeatureId, FlaggerStatus, capability};

use crate::{SourceSpan, Statement, error::ParserError};

/// One optional feature observed while walking an AST.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureUse {
    /// Generated capability identifier for an ISO feature or namespaced extension.
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
    let record = capability(id).expect("parser feature IDs are generated profile capabilities");
    if record.flagger_status == FlaggerStatus::Accepted {
        return Ok(());
    }
    Err(ParserError::UnsupportedFeature {
        feature_id: id,
        display_name: record.name,
        span,
        hint: record.non_support_rationale,
    })
}

/// Element-type DDL is not an ISO statement, so its `OR REPLACE` modifier has
/// no ISO semantics to implement. `CREATE OR REPLACE GRAPH` is ISO section
/// 12.4 and executes through the database facade; it is not rejected here.
fn reject_unimplemented(statement: &Statement) -> Result<(), ParserError> {
    if let Statement::Ddl(
        crate::DdlStatement::CreateNodeType {
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
            "OR REPLACE is defined by ISO/IEC 39075:2024 sections 12.4 and 12.6 for graph and graph-type statements; it is not implemented for element-type DDL, which is not an ISO statement",
            *span,
            Some("drop the modifier, or DROP and CREATE the element type explicitly"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_selected_parser_visible_capabilities_are_accepted() {
        let span = SourceSpan::new(7, 11);
        for id in [
            FeatureId::GC03,
            FeatureId::GE04,
            FeatureId::GE05,
            FeatureId::GH02,
            FeatureId::GG01,
            FeatureId::GG02,
            FeatureId::GG20,
            FeatureId::GG21,
            FeatureId::GS04,
            FeatureId::GV66,
            FeatureId::GV67,
        ] {
            check_feature(id, span).expect("direct selected parser surface is admitted");
        }
    }

    #[test]
    fn supported_unclaimed_iso_and_supported_extension_are_accepted() {
        for id in [FeatureId::GP04, FeatureId::IM_JSON] {
            check_feature(id, SourceSpan::default()).expect("supported capability is accepted");
        }
    }

    #[test]
    fn implied_unsupported_capability_is_rejected_with_generated_metadata() {
        let span = SourceSpan::new(7, 11);
        let ParserError::UnsupportedFeature {
            feature_id,
            display_name,
            span: rejected_span,
            hint,
        } = check_feature(FeatureId::GV65, span).expect_err("implied unsupported capability")
        else {
            panic!("expected unsupported-feature error");
        };
        assert_eq!(feature_id, FeatureId::GV65);
        assert_eq!(display_name, capability(FeatureId::GV65).unwrap().name);
        assert_eq!(rejected_span, span);
        assert_eq!(
            hint,
            capability(FeatureId::GV65).unwrap().non_support_rationale
        );
    }
}
