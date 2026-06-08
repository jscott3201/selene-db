//! ISO/IEC 39075:2024 GQL parser, AST, and Flagger for selene-db.
//!
//! See Spec 07 for the parser and Flagger design contract.

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
pub mod runtime;
mod temporal_parse;

pub use crate::analyze::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, BindingDecl,
    BindingDeclKind, BindingId, BindingScope, BindingScopeTree, BindingUse, BindingUseKind,
    ConditionClause, ElementKind, ExpectedType, ExprId, ExprIdLookup, ExprTypeTable,
    InvalidLabelForm, MutationWriteSet, ScopeId, ScopeKind, Side, StatementCategory,
    TypeMismatchContext, WriteKind, WriteSetEntry, analyze,
};
pub use crate::ast::{
    call::{InlineProcedureCall, ProcedureCall, YieldColumn, YieldItem},
    ddl::{
        DdlStatement, DropBehavior, EdgeEndpointSpec, KeyLabelSet, TypePropertyConstraint,
        TypePropertyDef, ValidationMode,
    },
    expr::{
        BinaryOp, DecimalLiteralKind, FloatLiteralKind, IntegerLiteralKind, IsCheckKind, Literal,
        NormalForm, TemporalDurationQualifier, TrimSpec, TruthValue, UnaryOp, ValueExpr,
    },
    format::format_procedure_call,
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
        QueryPipeline, ReturnClause, ReturnItem, SessionResetTarget, SetOp, Statement,
        UnwindStatement, WithClause,
    },
    types::{GqlType, RecordType},
    util::{EmptyVecError, NonEmpty, Vec2OrMore},
};
pub use crate::diagnostic::DiagnosticReport;
pub use crate::error::{GqlStatus, ParserError};
pub use crate::flagger::{FeatureUse, feature_walk};
pub use crate::parser::{parse, parse_many, parse_with_source};
pub use crate::plan::{
    Aggregate, AggregateArg, BindingDef, BindingElement, BindingTableColumn, BindingTableSchema,
    BuildSide, CatalogOp, CompositeIndexHandle, DeleteTargetPlan, EdgeMatch, EmptyIndexCatalog,
    ExecutionPlan, FilterPredicate, FilterPredicateKind, HiddenBindingId, HopContributor,
    ImplDefinedCaps, IndexCatalog, IndexHandle, IndexKey, IndexKind, IndexTarget,
    InsertEndpointRef, InsertSiteId, JoinTree, LimitAmount, LiveIndexCatalog, MutationOp,
    NodeIdOrdering, NodeOrEdgeScan, OptimizeContext, OrderAccess, OrderKey, OuterBindingRef,
    PathContributor, PathPlan, PatternPlan, PipelineOp, PipelineOpId, PlannedCall, PlannedSubquery,
    PlannedTableSubquery, PlannedTableSubqueryYield, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, PlannedYieldItem, PlannerError, ProjectExpr, PropertyInit,
    RepeatEdgeMatch, Rule, ScanAccess, ScanKind, SessionOp, SubqueryBody, SubqueryKind,
    SubqueryRegistry, TailBinding, Transformed, TxOp, TypedIndexBounds, TypedIndexLookup,
    YieldKind, optimize, plan,
};
pub use crate::procedure_registry::{
    EmptyProcedureRegistry, ProcedureArity, ProcedureDefaultValue, ProcedureError, ProcedureHandle,
    ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema,
    ProcedureParameter, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Value,
};
pub use crate::runtime::{
    AdaptiveOptimizer, Binding, BindingTable, BindingTableRegistry, BuiltinProcedureRegistry,
    CallPlanCache, CallPlanCacheStats, CallPlanKey, DataExceptionSubclass, ExecutorError,
    ExecutorWarning, GraphContext, MaintenanceContext, MutationContext, PlanCache, PlanCacheStats,
    ProcedureContext, RollbackOutcome, Session, SessionParameterValue, StatementOutput,
    TransactionOutcome, TxContext, WarningSink, WriteOutcome, execute_pattern, execute_pipeline,
    execute_statement,
};
pub use selene_core::{CancellationCause, CancellationChecker, CancellationToken};

#[cfg(any(test, feature = "test-harness"))]
pub use crate::runtime::{
    ExecutorSnapshot, ExecutorSummaryInput, NetGraphDelta, RowOrderPolicy, SnapshotColumn,
    executor_summary,
};
