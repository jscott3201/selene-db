//! Public GQL abstract syntax tree types.

pub mod call;
pub mod ddl;
pub mod eq;
pub mod expr;
pub mod format;
pub(crate) mod format_ident;
pub mod mutation;
pub mod pattern;
pub mod span;
pub mod statement;
pub mod types;
pub mod util;
mod walk;

pub use call::{InlineProcedureCall, ProcedureCall, YieldColumn, YieldItem};
pub use ddl::{
    DdlStatement, DropBehavior, EdgeEndpointSpec, KeyLabelSet, TypePropertyConstraint,
    TypePropertyDef, ValidationMode,
};
pub use eq::structurally_eq;
pub use expr::{
    BinaryOp, CharacterStringLiteralKind, DecimalLiteralKind, FloatLiteralKind, IntegerLiteralKind,
    IsCheckKind, Literal, NormalForm, TemporalDurationQualifier, TrimSpec, TruthValue, UnaryOp,
    ValueExpr,
};
pub use format::{FormatError, format_procedure_call, format_read_statement};
pub use mutation::{
    DeleteMode, DeleteStatement, InsertStatement, MutationPipeline, MutationStatement,
    MutationTerminator, RemoveItem, SetItem,
};
pub use pattern::{
    EdgeDirection, EdgePattern, GraphPattern, LabelExpr, MatchClause, MatchMode, NodePattern,
    PathMode, PathSelector, PatternElement, Quantifier,
};
pub use selene_core::{
    DecimalType, MAX_BYTE_STRING_TYPE_LENGTH, MAX_CHARACTER_STRING_TYPE_LENGTH,
    MAX_DECIMAL_PRECISION, MAX_DECIMAL_SCALE,
};
pub use span::SourceSpan;
pub use statement::{
    LetBinding, LimitValue, NullsPolicy, OrderDirection, OrderTerm, PipelineStatement,
    QueryPipeline, ReturnClause, ReturnItem, SessionResetTarget, SetOp, Statement, TypedBinding,
    UnwindStatement, WithClause,
};
pub use types::{
    BindingTableType, ByteStringType, ByteStringTypeForm, CharacterStringType,
    CharacterStringTypeForm, GqlType, RecordType,
};
pub use util::{EmptyVecError, NonEmpty, Vec2OrMore};
