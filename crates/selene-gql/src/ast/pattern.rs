//! Graph-pattern AST nodes.

use selene_core::IStr;

use crate::ast::{expr::ValueExpr, span::SourceSpan};

/// Path traversal mode for `MATCH`.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum PathMode {
    /// Edges and nodes may repeat.
    #[default]
    Walk,
    /// No repeated edges.
    Trail,
    /// No repeated nodes except allowed cycle endpoints.
    Acyclic,
    /// No repeated nodes.
    Simple,
}

/// Path selector.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PathSelector {
    /// `ANY`.
    Any,
    /// `ALL`.
    All,
    /// `ANY SHORTEST`.
    AnyShortest,
    /// `ALL SHORTEST`.
    AllShortest,
}

/// Match-mode modifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MatchMode {
    /// `DIFFERENT EDGES`.
    DifferentEdges,
    /// `REPEATABLE ELEMENTS`.
    RepeatableElements,
}

/// Label expression.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum LabelExpr {
    /// One label name.
    Single(IStr),
    /// Conjunction.
    Conjunction(Vec<LabelExpr>),
    /// Disjunction.
    Disjunction(Vec<LabelExpr>),
    /// Negation.
    Negation(Box<LabelExpr>),
    /// Wildcard label.
    Wildcard,
}

/// Direction requested by an edge pattern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum EdgeDirection {
    /// `-[]->`.
    Right,
    /// `<-[]-`.
    Left,
    /// `-[]-`, matching either direction.
    Undirected,
}

/// Variable-length path quantifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Quantifier {
    /// Minimum number of repetitions.
    pub min: u32,
    /// Maximum number of repetitions, or `None` for unbounded.
    pub max: Option<u32>,
}

/// Node pattern.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct NodePattern {
    /// Optional binding.
    pub binding: Option<IStr>,
    /// Optional label expression.
    pub label_expr: Option<LabelExpr>,
    /// Inline property predicates in source order.
    pub properties: Vec<(IStr, ValueExpr)>,
    /// Optional inline `WHERE`.
    pub inline_where: Option<ValueExpr>,
    /// Source span.
    pub span: SourceSpan,
}

/// Edge pattern.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EdgePattern {
    /// Optional binding.
    pub binding: Option<IStr>,
    /// Direction.
    pub direction: EdgeDirection,
    /// Optional label expression.
    pub label_expr: Option<LabelExpr>,
    /// Inline property predicates in source order.
    pub properties: Vec<(IStr, ValueExpr)>,
    /// Optional variable-length quantifier.
    pub quantifier: Option<Quantifier>,
    /// Optional inline `WHERE`.
    pub inline_where: Option<ValueExpr>,
    /// Source span.
    pub span: SourceSpan,
}

/// One graph-pattern element.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum PatternElement {
    /// Node pattern.
    Node(NodePattern),
    /// Edge pattern.
    Edge(EdgePattern),
}

/// One graph pattern.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct GraphPattern {
    /// Optional path binding.
    pub path_binding: Option<IStr>,
    /// Alternating node/edge/node elements.
    pub elements: Vec<PatternElement>,
    /// Source span.
    pub span: SourceSpan,
}

/// `MATCH` clause.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MatchClause {
    /// Whether this is `OPTIONAL MATCH`.
    pub optional: bool,
    /// Optional path selector.
    pub selector: Option<PathSelector>,
    /// Optional match mode.
    pub match_mode: Option<MatchMode>,
    /// Path mode; defaults to [`PathMode::Walk`].
    pub path_mode: PathMode,
    /// Graph patterns.
    pub patterns: Vec<GraphPattern>,
    /// Optional statement-level `WHERE`.
    pub where_clause: Option<ValueExpr>,
    /// Source span.
    pub span: SourceSpan,
}
