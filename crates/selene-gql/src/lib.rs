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
pub mod plan;
pub mod procedure_registry;

pub use crate::analyze::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, BindingDecl,
    BindingDeclKind, BindingId, BindingScope, BindingScopeTree, BindingUse, BindingUseKind,
    ConditionClause, ElementKind, ExpectedType, ExprId, ExprIdMap, ExprTypeTable, InvalidLabelForm,
    MutationWriteSet, ScopeId, ScopeKind, Side, StatementCategory, TypeMismatchContext, WriteKind,
    WriteSetEntry, analyze,
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
pub use crate::plan::{
    Aggregate, AggregateArg, BindingDef, BindingElement, BindingTableColumn, BindingTableSchema,
    CatalogOp, EdgeMatch, EdgeStatistics, ExecutionPlan, FilterPredicate, FilterPredicateKind,
    ImplDefinedCaps, InsertEndpointRef, InsertSiteId, JoinTree, LimitAmount, MutationOp,
    NodeOrEdgeScan, OptimizeContext, OrderKey, PathPlan, PatternPlan, PipelineOp, PlannedCall,
    PlannedTypePropertyConstraint, PlannedTypePropertyDef, PlannedYieldItem, PlannerError,
    ProjectExpr, PropertyHistogram, PropertyInit, Rule, ScanKind, Transformed, TxOp,
    WanderJoinSampler, YieldKind, optimize, plan,
};
pub use crate::procedure_registry::{
    EmptyProcedureRegistry, ProcedureHandle, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputColumn, ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry,
    ProcedureSignature, ProcedureTier,
};
