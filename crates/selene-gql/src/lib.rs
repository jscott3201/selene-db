//! ISO/IEC 39075:2024 GQL parser, AST, and Flagger for selene-db.
//!
//! See `_spec/07-iso-gql-parser-and-flagger.md` for the design contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod analyze;
pub mod ast;
pub mod diagnostic;
pub mod error;
mod flagger;
pub mod parser;

pub use crate::analyze::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, BindingDecl,
    BindingDeclKind, BindingId, BindingScope, BindingScopeTree, BindingUse, BindingUseKind,
    ScopeId, ScopeKind, analyze,
};
pub use crate::ast::{
    call::{ProcedureCall, YieldColumn, YieldItem},
    ddl::{
        DdlStatement, EdgeEndpointSpec, TypePropertyConstraint, TypePropertyDef, ValidationMode,
    },
    expr::{BinaryOp, IsCheckKind, Literal, NormalForm, TruthValue, UnaryOp, ValueExpr},
    mutation::{
        DeleteMode, DeleteStatement, InsertStatement, MutationPipeline, MutationStatement,
        MutationTerminator, RemoveItem, SetItem,
    },
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
pub use crate::diagnostic::DiagnosticReport;
pub use crate::error::{GqlStatus, ParserError};
pub use crate::flagger::{FeatureUse, feature_walk};
pub use crate::parser::{parse, parse_with_source};
