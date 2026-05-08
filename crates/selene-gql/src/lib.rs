//! ISO/IEC 39075:2024 GQL parser, AST, and Flagger for selene-db.
//!
//! See `_spec/07-iso-gql-parser-and-flagger.md` for the design contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod ast;
pub mod error;
pub mod parser;

pub use crate::ast::{
    expr::{BinaryOp, IsCheckKind, Literal, NormalForm, TruthValue, UnaryOp, ValueExpr},
    pattern::{
        EdgeDirection, EdgePattern, GraphPattern, LabelExpr, MatchClause, MatchMode, NodePattern,
        PathMode, PathSelector, PatternElement, Quantifier,
    },
    span::SourceSpan,
    statement::{
        LetBinding, LimitValue, NullsPolicy, OrderDirection, OrderTerm, PipelineStatement,
        QueryPipeline, ReturnClause, ReturnItem, SetOp, Statement, UnwindStatement, WithClause,
    },
    types::{GqlType, RecordType, ValueType},
};
pub use crate::error::{GqlStatus, ParserError};
pub use crate::parser::parse;
