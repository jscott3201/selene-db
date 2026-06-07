//! Binding helpers for singleton graph element variable references.

use crate::{
    GqlType, ValueExpr,
    analyze::{
        error::AnalysisError,
        types::{AnalyzedType, ExprId},
        write_set::ElementKind,
    },
};

use super::{BindContext, expr::bind_value_expr};

pub(crate) fn bind_singleton_element_variable_references(
    ctx: &mut BindContext<'_>,
    targets: &[ValueExpr],
    context: &'static str,
) -> Result<(), AnalysisError> {
    for target in targets {
        bind_singleton_element_variable_reference(
            ctx,
            target,
            ElementReferenceRequirement::NodeOrEdgePlural,
            context,
        )?;
    }
    Ok(())
}

pub(crate) fn bind_singleton_element_variable_reference(
    ctx: &mut BindContext<'_>,
    target: &ValueExpr,
    requirement: ElementReferenceRequirement,
    context: &'static str,
) -> Result<(), AnalysisError> {
    let target_id = bind_value_expr(ctx, target)?;
    validate_singleton_element_variable_reference(ctx, target, target_id, requirement, context)
}

pub(crate) fn validate_singleton_element_variable_reference(
    ctx: &BindContext<'_>,
    target: &ValueExpr,
    target_id: ExprId,
    requirement: ElementReferenceRequirement,
    context: &'static str,
) -> Result<(), AnalysisError> {
    let ValueExpr::Variable { name, span } = target else {
        return Err(invalid_element_reference(
            context,
            requirement,
            target.span(),
        ));
    };
    let binding = ctx
        .lookup_binding(name)
        .expect("element variable target was just resolved");
    let target_type = ctx.expr_type(target_id);
    if requirement.matches(ctx.element_kind(binding), target_type) {
        Ok(())
    } else {
        Err(invalid_element_reference(context, requirement, *span))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ElementReferenceRequirement {
    Node,
    Edge,
    NodeOrEdge,
    NodeOrEdgePlural,
}

impl ElementReferenceRequirement {
    fn matches(self, kind: ElementKind, ty: &AnalyzedType) -> bool {
        let node = matches!(
            (kind, ty),
            (ElementKind::Node, AnalyzedType::Resolved(GqlType::NodeRef))
        );
        let edge = matches!(
            (kind, ty),
            (ElementKind::Edge, AnalyzedType::Resolved(GqlType::EdgeRef))
        );
        match self {
            Self::Node => node,
            Self::Edge => edge,
            Self::NodeOrEdge | Self::NodeOrEdgePlural => node || edge,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Node => "a singleton node variable reference",
            Self::Edge => "a singleton edge variable reference",
            Self::NodeOrEdge => "a singleton node or edge variable reference",
            Self::NodeOrEdgePlural => "singleton node or edge variable references",
        }
    }
}

fn invalid_element_reference(
    context: &'static str,
    requirement: ElementReferenceRequirement,
    span: crate::SourceSpan,
) -> AnalysisError {
    AnalysisError::InvalidReference {
        message: format!("{context} must be {}", requirement.description()),
        span,
    }
}
