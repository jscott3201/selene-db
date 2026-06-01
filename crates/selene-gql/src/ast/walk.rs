//! Shared structural walkers for [`ValueExpr`].
//!
//! Hand-maintained exhaustive `match` arms over [`ValueExpr`] were duplicated
//! across the analyzer, planner, optimizer, and parser — every new variant
//! forced an edit in each copy, and a missed arm was a silent traversal bug.
//! This module centralizes the child- and span-enumeration shape in three
//! inherent methods so adopters express only their per-node work and delegate
//! traversal here:
//!
//! - [`ValueExpr::for_each_child`] / [`ValueExpr::for_each_child_mut`] visit each
//!   **direct** child `ValueExpr` of a node, in source order. They are *shallow*:
//!   the callback receives each immediate child once, and the caller drives
//!   recursion by re-invoking itself inside the callback. Subquery nodes
//!   ([`ValueExpr::Exists`], [`ValueExpr::CountSubquery`],
//!   [`ValueExpr::ValueSubquery`]) carry no direct `ValueExpr` children — their
//!   bodies are [`MatchClause`](crate::MatchClause) /
//!   [`QueryPipeline`](crate::QueryPipeline) — so they yield nothing and the
//!   caller handles any subquery descent explicitly.
//! - [`ValueExpr::for_each_span_mut`] visits the [`SourceSpan`] **owned directly
//!   by this node** (for [`ValueExpr::Literal`] the span stored inside the
//!   [`Literal`](crate::Literal)). It is likewise shallow; spans of child nodes
//!   are reached by recursing with [`for_each_child_mut`](ValueExpr::for_each_child_mut).
//!
//! The shallow contract keeps the methods variant-additive-safe (a new variant
//! is handled in exactly one place) while preserving each adopter's existing
//! recursion order and side-effect timing byte-for-byte.

use crate::ast::{ValueExpr, expr::IsCheckKind, span::SourceSpan};

impl ValueExpr {
    /// Visit each direct child [`ValueExpr`] of this node, in source order.
    ///
    /// Shallow: the callback receives each immediate child exactly once and is
    /// **not** re-applied to grandchildren. Callers that want a full traversal
    /// re-invoke themselves inside `f`. The
    /// [`IsCheckKind::SourceOf`]/[`IsCheckKind::DestinationOf`] operand counts as
    /// a child of the enclosing [`ValueExpr::IsCheck`] and is yielded after the
    /// checked `operand`. Subquery variants yield no children.
    pub fn for_each_child(&self, f: &mut impl FnMut(&ValueExpr)) {
        match self {
            Self::Literal(_) | Self::Variable { .. } | Self::Parameter { .. } => {}
            Self::PropertyAccess { target, .. } | Self::PropertyExists { target, .. } => f(target),
            Self::ListAccess { target, index, .. } => {
                f(target);
                f(index);
            }
            Self::ListLiteral { items, .. }
            | Self::AllDifferent { items, .. }
            | Self::Same { items, .. } => {
                for item in items {
                    f(item);
                }
            }
            Self::RecordLiteral { fields, .. } => {
                for (_, value) in fields {
                    f(value);
                }
            }
            Self::BinaryOp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Self::UnaryOp { operand, .. } => f(operand),
            Self::FunctionCall { args, .. } => {
                for arg in args {
                    f(arg);
                }
            }
            Self::Normalize { source, .. } => f(source),
            Self::Trim {
                character, source, ..
            } => {
                if let Some(character) = character {
                    f(character);
                }
                f(source);
            }
            Self::IsCheck { operand, kind, .. } => {
                f(operand);
                if let IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) = kind {
                    f(value);
                }
            }
            Self::InList { operand, list, .. } => {
                f(operand);
                for item in list {
                    f(item);
                }
            }
            Self::Case {
                branches,
                else_branch,
                ..
            } => {
                for (condition, value) in branches {
                    f(condition);
                    f(value);
                }
                if let Some(value) = else_branch {
                    f(value);
                }
            }
            Self::Cast { value, .. } => f(value),
            // Subquery bodies are `MatchClause` / `QueryPipeline`, not
            // `ValueExpr`; they carry no direct `ValueExpr` children.
            Self::Exists { .. } | Self::CountSubquery { .. } | Self::ValueSubquery { .. } => {}
        }
    }

    /// Visit each direct child [`ValueExpr`] of this node mutably, in source
    /// order.
    ///
    /// Mutable mirror of [`for_each_child`](Self::for_each_child); the same
    /// shallow contract and child ordering apply.
    pub fn for_each_child_mut(&mut self, f: &mut impl FnMut(&mut ValueExpr)) {
        match self {
            Self::Literal(_) | Self::Variable { .. } | Self::Parameter { .. } => {}
            Self::PropertyAccess { target, .. } | Self::PropertyExists { target, .. } => f(target),
            Self::ListAccess { target, index, .. } => {
                f(target);
                f(index);
            }
            Self::ListLiteral { items, .. }
            | Self::AllDifferent { items, .. }
            | Self::Same { items, .. } => {
                for item in items {
                    f(item);
                }
            }
            Self::RecordLiteral { fields, .. } => {
                for (_, value) in fields {
                    f(value);
                }
            }
            Self::BinaryOp { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Self::UnaryOp { operand, .. } => f(operand),
            Self::FunctionCall { args, .. } => {
                for arg in args {
                    f(arg);
                }
            }
            Self::Normalize { source, .. } => f(source),
            Self::Trim {
                character, source, ..
            } => {
                if let Some(character) = character {
                    f(character);
                }
                f(source);
            }
            Self::IsCheck { operand, kind, .. } => {
                f(operand);
                if let IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) = kind {
                    f(value);
                }
            }
            Self::InList { operand, list, .. } => {
                f(operand);
                for item in list {
                    f(item);
                }
            }
            Self::Case {
                branches,
                else_branch,
                ..
            } => {
                for (condition, value) in branches {
                    f(condition);
                    f(value);
                }
                if let Some(value) = else_branch {
                    f(value);
                }
            }
            Self::Cast { value, .. } => f(value),
            // Subquery bodies are `MatchClause` / `QueryPipeline`, not
            // `ValueExpr`; they carry no direct `ValueExpr` children.
            Self::Exists { .. } | Self::CountSubquery { .. } | Self::ValueSubquery { .. } => {}
        }
    }

    /// Visit the [`SourceSpan`] owned directly by this node.
    ///
    /// Shallow: only the span(s) belonging to *this* node are visited (for
    /// [`ValueExpr::Literal`] the span stored inside the
    /// [`Literal`](crate::Literal)). Spans of child expressions are reached by
    /// recursing with [`for_each_child_mut`](Self::for_each_child_mut). Every
    /// `ValueExpr` variant owns exactly one span, so the callback fires once per
    /// node.
    pub fn for_each_span_mut(&mut self, f: &mut impl FnMut(&mut SourceSpan)) {
        match self {
            Self::Literal(literal) => f(literal.span_mut()),
            Self::Variable { span, .. }
            | Self::Parameter { span, .. }
            | Self::PropertyAccess { span, .. }
            | Self::ListAccess { span, .. }
            | Self::ListLiteral { span, .. }
            | Self::RecordLiteral { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::FunctionCall { span, .. }
            | Self::IsCheck { span, .. }
            | Self::InList { span, .. }
            | Self::AllDifferent { span, .. }
            | Self::Same { span, .. }
            | Self::PropertyExists { span, .. }
            | Self::Case { span, .. }
            | Self::Exists { span, .. }
            | Self::CountSubquery { span, .. }
            | Self::ValueSubquery { span, .. }
            | Self::Normalize { span, .. }
            | Self::Trim { span, .. }
            | Self::Cast { span, .. } => f(span),
        }
    }
}

#[cfg(test)]
mod tests {
    use selene_core::IStr;

    use crate::ast::ValueExpr;
    use crate::ast::expr::{BinaryOp, IsCheckKind, Literal};
    use crate::ast::span::SourceSpan;

    fn span(offset: u32) -> SourceSpan {
        SourceSpan::new(offset, 1)
    }

    fn istr(value: &str) -> IStr {
        selene_core::intern(value).expect("test string interns")
    }

    fn var(name: &str, offset: u32) -> ValueExpr {
        ValueExpr::Variable {
            name: istr(name),
            span: span(offset),
        }
    }

    fn int(value: i64, offset: u32) -> ValueExpr {
        ValueExpr::Literal(Literal::Integer(value, span(offset)))
    }

    #[test]
    fn for_each_child_binary_op_yields_both_operands_in_order() {
        let expr = ValueExpr::BinaryOp {
            op: BinaryOp::Add,
            lhs: Box::new(int(1, 10)),
            rhs: Box::new(int(2, 20)),
            span: span(0),
        };
        let mut seen = Vec::new();
        expr.for_each_child(&mut |child| seen.push(child.span().byte_offset));
        assert_eq!(seen, vec![10, 20]);
    }

    #[test]
    fn for_each_child_list_literal_yields_every_item() {
        let expr = ValueExpr::ListLiteral {
            items: vec![int(1, 1), int(2, 2), int(3, 3)],
            span: span(0),
        };
        let mut seen = Vec::new();
        expr.for_each_child(&mut |child| seen.push(child.span().byte_offset));
        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[test]
    fn for_each_child_case_yields_branches_then_else() {
        let expr = ValueExpr::Case {
            branches: vec![(int(1, 1), int(2, 2)), (int(3, 3), int(4, 4))],
            else_branch: Some(Box::new(int(5, 5))),
            span: span(0),
        };
        let mut seen = Vec::new();
        expr.for_each_child(&mut |child| seen.push(child.span().byte_offset));
        assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn for_each_child_is_check_yields_operand_then_kind_value() {
        let expr = ValueExpr::IsCheck {
            operand: Box::new(var("n", 1)),
            kind: IsCheckKind::SourceOf(Box::new(var("e", 2))),
            negated: false,
            span: span(0),
        };
        let mut seen = Vec::new();
        expr.for_each_child(&mut |child| seen.push(child.span().byte_offset));
        assert_eq!(seen, vec![1, 2]);
    }

    #[test]
    fn for_each_child_trim_skips_absent_character() {
        let expr = ValueExpr::Trim {
            spec: crate::ast::expr::TrimSpec::Both,
            character: None,
            source: Box::new(var("s", 7)),
            span: span(0),
        };
        let mut seen = Vec::new();
        expr.for_each_child(&mut |child| seen.push(child.span().byte_offset));
        assert_eq!(seen, vec![7]);
    }

    #[test]
    fn for_each_child_subquery_yields_nothing() {
        // VALUE { ... } carries a QueryPipeline body, not a ValueExpr child.
        let pipeline = crate::parse("RETURN 1")
            .ok()
            .and_then(|statement| match statement {
                crate::Statement::Query(pipeline) => Some(pipeline),
                _ => None,
            })
            .expect("RETURN 1 parses to a query pipeline");
        let expr = ValueExpr::ValueSubquery {
            body: Box::new(pipeline),
            span: span(0),
        };
        let mut count = 0;
        expr.for_each_child(&mut |_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn for_each_child_mut_can_rewrite_children() {
        let mut expr = ValueExpr::BinaryOp {
            op: BinaryOp::Add,
            lhs: Box::new(var("a", 1)),
            rhs: Box::new(var("b", 2)),
            span: span(0),
        };
        // Replace every direct child variable with a literal.
        expr.for_each_child_mut(&mut |child| {
            if matches!(child, ValueExpr::Variable { .. }) {
                *child = int(0, 99);
            }
        });
        let mut kinds = Vec::new();
        expr.for_each_child(&mut |child| kinds.push(matches!(child, ValueExpr::Literal(_))));
        assert_eq!(kinds, vec![true, true]);
    }

    #[test]
    fn for_each_span_mut_visits_only_the_owning_node_span() {
        // The node span is offset 0; the child spans (10, 20) must be untouched
        // because the walker is shallow.
        let mut expr = ValueExpr::BinaryOp {
            op: BinaryOp::Add,
            lhs: Box::new(int(1, 10)),
            rhs: Box::new(int(2, 20)),
            span: span(0),
        };
        let mut visited = Vec::new();
        expr.for_each_span_mut(&mut |s| {
            visited.push(s.byte_offset);
            s.byte_offset = s.byte_offset.saturating_add(100);
        });
        assert_eq!(visited, vec![0]);
        let ValueExpr::BinaryOp { span, lhs, rhs, .. } = &expr else {
            unreachable!("constructed a BinaryOp");
        };
        assert_eq!(span.byte_offset, 100);
        assert_eq!(lhs.span().byte_offset, 10);
        assert_eq!(rhs.span().byte_offset, 20);
    }

    #[test]
    fn for_each_span_mut_reaches_literal_inner_span() {
        let mut expr = int(7, 42);
        let mut visited = Vec::new();
        expr.for_each_span_mut(&mut |s| visited.push(s.byte_offset));
        assert_eq!(visited, vec![42]);
    }

    #[test]
    fn recursive_span_walk_offsets_every_span() {
        // Composing for_each_span_mut with for_each_child_mut recursion mirrors
        // the parser's rebase / the eq scrub: every span in the tree is visited
        // exactly once.
        fn offset_all(expr: &mut ValueExpr, by: u32) {
            expr.for_each_span_mut(&mut |s| s.byte_offset = s.byte_offset.saturating_add(by));
            expr.for_each_child_mut(&mut |child| offset_all(child, by));
        }
        let mut expr = ValueExpr::BinaryOp {
            op: BinaryOp::Add,
            lhs: Box::new(int(1, 10)),
            rhs: Box::new(ValueExpr::UnaryOp {
                op: crate::ast::expr::UnaryOp::Negate,
                operand: Box::new(int(2, 30)),
                span: span(20),
            }),
            span: span(0),
        };
        offset_all(&mut expr, 1000);
        let mut all = Vec::new();
        fn gather(expr: &ValueExpr, out: &mut Vec<u32>) {
            let mut e = expr.clone();
            e.for_each_span_mut(&mut |s| out.push(s.byte_offset));
            expr.for_each_child(&mut |child| gather(child, out));
        }
        gather(&expr, &mut all);
        all.sort_unstable();
        assert_eq!(all, vec![1000, 1010, 1020, 1030]);
    }
}
