# Selene DB 2.0 work items M05–M10

<!-- Generated from plan.json; do not edit by hand. -->

The machine plan carries additional design, path, documentation, and benchmark metadata for each contract.

<a id="m05-pr01"></a>
## M05-PR01 — Separate the Source Syntax AST from the Annotated Semantic Tree

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M02-PR04, M03-PR05
- **Issues:** None
- **Commit scope:** `gql`

Establish immutable syntax and semantic representations so parsing preserves source while analysis produces resolved, typed, profile-aware objects without mutating parser nodes.

### Scope

- Define a source-oriented syntax AST with stable source spans, spelling, token distinctions, and no resolved catalog/runtime handles.
- Define an immutable annotated semantic tree (AST/HIR) with semantic node IDs, resolved references, inferred/declared types, effects, scope/degree metadata, and feature/evidence annotations.
- Refactor analyzer entry points to return the semantic tree plus diagnostics rather than a partially annotated syntax AST.
- Add deterministic semantic ID allocation and canonical debug/snapshot formatting.
- Preserve parser error and source span quality; do not conflate parser and analyzer diagnostics.
- Create a temporary lowering adapter from the semantic tree to the current execution plan.

### Non-goals

- No complete name/type/effect implementation; subsequent PRs fill those fields.
- No parser grammar rewrite.
- No physical or batch plan.
- No public serialization guarantee for internal AST/HIR.

### Acceptance evidence

- No analyzer code mutates syntax AST fields to store resolution/type state.
- Positive corpus produces deterministic semantic snapshots across runs.
- Parser-only tests remain independent of catalog/session setup.
- Source spans and diagnostics for existing negative corpus are preserved or deliberately improved with reviewed snapshots.
- Temporary old-plan adapter consumes semantic nodes, not the original syntax tree.
- Semantic tree records profile/catalog/session generation dependencies.

### Tests and gates

- Parser AST golden snapshots.
- Semantic-tree golden snapshots over representative statements.
- Determinism tests under randomized hash seeds/order.
- Origin/span mapping tests through syntactic transforms.
- Existing positive/negative corpus parity.
- Mutation tests around pass selection and semantic ID allocation.

### Review focus

- No semantic state in syntax AST.
- Deterministic IDs/snapshots.
- Diagnostic/source preservation.
- Bridge is one-way and temporary.

### Stop conditions

- Parser and analyzer types are too entangled to split within size cap; extract one statement family first and preserve end-state contract.
- Snapshot format includes nondeterministic internal details.
- The bridge starts depending on syntax nodes directly.

### Bridge and deletion

- Current execution plan lowering remains behind a semantic-tree adapter.
- Delete direct `AnalyzedStatement`/syntax mixed types by M05-PR06.

<a id="m05-pr02"></a>
## M05-PR02 — Implement Catalog-Aware Name, Scope, and Reference Resolution

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M05-PR01, M04-PR01
- **Issues:** None
- **Commit scope:** `gql`

Resolve schemas, graphs, graph types, procedures, variables, parameters, labels, properties, and working scopes to stable semantic identities with exact namespace and visibility rules.

### Scope

- Define analyzer scope frames and disjoint namespaces for binding variables, parameters, graph pattern variables, catalog objects, labels, and properties.
- Resolve absolute/relative schema and object references using session/current working schema and nested scope clauses.
- Resolve graph references/sites, including current session graph and USE GRAPH focus, to stable graph IDs/reference descriptors; implement full AT SCHEMA and USE GRAPH lexical scope/site semantics deferred by M03-PR05.
- Resolve variable declarations/uses, multiply declared element variables, path/subpath variables, degree of reference, and shadowing rules.
- Resolve procedure calls against catalog/native registry generation and argument names/signatures.
- Emit structured ambiguity, not-found, duplicate, out-of-scope, invalid-reference, and access diagnostics with source spans.

### Non-goals

- No full type inference.
- No optimizer/index lookup.
- No privilege model beyond invoking the M03 policy hook.
- No nested catalog directories.

### Acceptance evidence

- Resolution corpus covers absolute/relative catalog references, nested working scopes, variables, parameters, pattern joins, and procedure calls.
- Drop/recreate same-name objects cannot make an old semantic tree refer to the new object.
- Ambiguous or illegal references produce exact diagnostics at the narrowest source span.
- No runtime operator performs ordinary name lookup that could have been resolved statically.
- Visibility/access checks use one catalog/policy service.
- Resolution snapshots are deterministic across catalog insertion order.

### Tests and gates

- Table-driven namespace/scope tests.
- Catalog drop/recreate/generation invalidation tests.
- Pattern-variable degree/multiple-declaration tests.
- Procedure overload/signature resolution tests.
- Negative diagnostic snapshots.
- Property tests for scope push/pop and canonical name lookup.

### Review focus

- Stable IDs after resolution.
- Scope and namespace correctness.
- Generation/access dependencies.
- No runtime string resolution.

### Stop conditions

- Procedure catalog ownership from M10 is needed for current calls; use a typed native registry adapter with generation, not raw strings.
- A reference form depends on an unselected feature; flag it instead of half-resolving.
- Name canonicalization differs from catalog service.

### Bridge and deletion

- Native procedure registry adapter is temporary until M10-PR04.
- Current runtime name lookups are deleted as each semantic family migrates.

<a id="m05-pr03"></a>
## M05-PR03 — Implement Structural Type Descriptors, Nullability, References, and Binding Table Schemas

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M05-PR02, M03-PR03
- **Issues:** None
- **Commit scope:** `types`

Replace scattered expression/value type enums with interned structural descriptors that can represent the selected GQL value, record, list, path, database-scoped reference, object, and binding-table types exactly enough for annotation and results, and migrate semantic/runtime reference carriers off the M04 legacy bare-ID bridge.

### Scope

- Define interned immutable data/base/value/object/reference/list/record/path/binding-table descriptors with material/nullable/immaterial and open/closed characteristics.
- Define assignability, comparability, identity/distinct/equality/order capability, field combination/amend/restrict, list element/cardinality, and type-normal-form operations for the selected profile.
- Represent graph/node/edge/table reference types with optional constraining object types and stable descriptor IDs; migrate request/result/runtime carriers to M04's database-scoped facade reference contract rather than treating legacy bare-ID `Value` variants as complete references.
- Build binding-table schemas from closed record types plus preferred column order and ordered/unordered metadata.
- Refactor parameter/property/procedure/result typing to use descriptors or explicit adapters.
- Keep unsupported dynamic-union or optional types truthful in the profile rather than approximating them with `Any`.

### Non-goals

- No full implementation of every optional GQL type.
- No physical column vector layout.
- No persisted type encoding.
- No nominal user-defined type system.

### Acceptance evidence

- Equivalent type specifications intern to the same canonical descriptor and incompatible types remain distinct.
- Assignability/comparability/equality/order tests cover all selected scalar, reference, list, record, path, null, and dynamic/open cases.
- Binding-table schema preserves field names/types, ordered flag, duplicates, and preferred output column order.
- Parameter/property/procedure/result type checks use one type service.
- Unsupported types/features fail with feature/type diagnostics rather than silently widening.
- Conformance evidence links to pure type-model tests.

### Tests and gates

- Exhaustive small type-lattice/model tests.
- Property tests for normalization idempotence and assignability transitivity where applicable.
- Record/list/reference descriptor tests.
- Null/omitted/distinct/equality/order truth tables.
- Existing expression/property/parameter tests through adapters.
- Mutation tests for type relation branches.

### Review focus

- No catch-all type that hides unsupported semantics.
- Null versus omitted correctness.
- Canonical normal forms and pure operations.
- Reference constraints and binding-table schema.

### Stop conditions

- The selected profile requires an optional type family not yet scheduled; update profile/roadmap before adding a shortcut.
- Interning lifetime leaks across database/profile boundaries.
- Current Value representation cannot preserve required distinctions; split a core value prerequisite.

### Bridge and deletion

- Old expression/type enums may convert to descriptors temporarily.
- Delete semantic/runtime dependence on legacy bare-ID `selene_core::Value` reference variants after migrating request/result carriers to M04 facade references; M09-PR08 retains ownership of encoded variant/codec deletion.
- Delete adapters in M05-PR06 and M06 as physical columns land.

<a id="m05-pr04"></a>
## M05-PR04 — Implement Statement Effects, Procedure Side-Effect Classes, and Transaction Write Sets

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M05-PR02, M05-PR03, M03-PR04
- **Issues:** None
- **Commit scope:** `gql`

Annotate every command/procedure/statement with catalog, data, session, transaction, or query effects and compute precise write sets for validation, privilege checks, planning, and cache safety.

### Scope

- Define effect categories and compositional effect summaries for programs, procedures, commands, statements, expressions, and procedure calls.
- Compute catalog object, graph, label/type/property/index/constraint targets and unknown/dynamic write-set markers.
- Validate read-only transaction restrictions, catalog/data mixing profile, query-procedure purity, and procedure descriptor declarations.
- Feed authorization checks, transaction preparation, plan cache policy, and mutation operator lowering from the same effect summary.
- Reject or conservatively classify dynamic procedure calls/graph references that cannot prove a narrow write set.
- Add evidence records for side-effect and statement-category semantics.

### Non-goals

- No physical mutation execution.
- No row-level conflict detection/MVCC.
- No full procedure catalog definition.
- No optimizer cost model.

### Acceptance evidence

- Every supported statement/procedure family has a deterministic effect summary and write set.
- Read-only/mixing/purity violations are caught during annotation with expected diagnostics.
- Authorization and transaction code consume the same summary rather than reclassifying syntax.
- Combining nested/composite operations preserves monotonicity in property tests.
- Dynamic/unknown cases are conservative and visible in debug output.
- Plan cache never reuses a plan under incompatible effect/catalog dependencies.

### Tests and gates

- Table-driven effect tests for all statement families.
- Lattice/property tests.
- Read-only and mixing policy negative tests.
- Native procedure descriptor mismatch tests.
- Plan-cache policy tests.
- Mutation tests around category classification.

### Review focus

- One classification source.
- Conservative unknown behavior.
- Transaction/profile integration.
- Stable-ID write sets.

### Stop conditions

- A current procedure has undocumented side effects incompatible with its descriptor; fix descriptor/behavior first.
- Write-set precision requires executing user code.
- Authorization checks remain syntax-specific.

### Bridge and deletion

- Old `StatementCategory` may wrap the effect summary temporarily.
- Delete duplicate classifiers in M05-PR06.

<a id="m05-pr05"></a>
## M05-PR05 — Introduce the Logical Binding-Table and Graph Algebra IR

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M05-PR03, M05-PR04
- **Issues:** None
- **Commit scope:** `planner`

Lower the semantic tree into a typed logical plan that expresses GQL dataflow, graph matching, mutations, DDL, procedures, and transaction commands without physical storage choices.

### Scope

- Define typed logical operators for unit/empty tables, graph scan/match, filter, let/for, project/select/return, join/product, aggregate/group, distinct, sort, offset/limit/page, union/composition, procedure call, data mutation, catalog mutation, session/transaction command, and finish/omitted result.
- Define logical expression IR referencing semantic expression/type IDs and stable catalog objects.
- Carry input/output binding-table descriptors, ordering/duplicate guarantees, working graph/schema IDs, effects, and source origins on each operator.
- Lower supported semantic statements deterministically and reject unsupported/profile-gated forms before physical planning.
- Implement semantics-preserving logical normalization rules only, separate from cost/physical optimization.
- Generate compact deterministic plan snapshots and a visitor framework.

### Non-goals

- No index choice, join algorithm, batch size, row/bitmap representation, or path traversal algorithm.
- No physical executor.
- No generalized DAG sharing if a tree/arena suffices initially.
- No JIT/code generation.

### Acceptance evidence

- All currently supported statement categories lower to valid logical plans or explicit unsupported diagnostics.
- Plan validator detects schema, type, graph, order, duplicate, effect, and disconnected-node inconsistencies.
- Golden snapshots are deterministic and readable enough for PR review.
- Logical plans contain no `RowIndex`, `RoaringBitmap`, concrete index, storage map, or executor closure.
- Result descriptors/outcomes can be derived from the plan root.
- Existing query outputs remain reachable through the temporary physical adapter.

### Tests and gates

- Golden logical plan snapshots across statement families.
- Plan validation negative fixtures.
- Lowering differential tests against semantic result/effect descriptors.
- Random small-plan validator property tests.
- Existing corpus through old physical adapter.
- Mutation tests for logical normalization/validation.

### Review focus

- No physical leakage.
- Exact schema/order/duplicate/effect properties.
- Deterministic lowering/validation.
- Mutation sink semantics.

### Stop conditions

- An operator cannot express required semantics without a physical field; revise semantic property, not leak storage choice.
- Current path semantics require too much detail; use opaque typed path sub-IR until M07.
- Plan snapshots are unstable or too verbose to review.

### Bridge and deletion

- Old `ExecutionPlan` becomes a physical-adapter output, not the logical source of truth.
- M05-PR06 deletes direct semantic→old-plan lowering.

<a id="m05-pr06"></a>
## M05-PR06 — Unify Compiler Diagnostics and Cut Over to Semantic→Logical Planning

- **Owner:** M05
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M05-PR05
- **Issues:** None
- **Commit scope:** `gql`

Complete the compiler boundary by routing parse, annotation, type, effect, and logical validation diagnostics through structured GQL status objects and deleting the old mixed analyzed/plan path.

### Scope

- Define one compiler diagnostic type with phase, primary GQLSTATUS, additional/nested statuses, source labels, semantic/plan IDs, feature/evidence IDs, and safe debug context.
- Map parser/analyzer/type/effect/logical validation errors to exact statuses and precedence rules.
- Update request execution to perform source→syntax→semantic→logical stages and return failed RequestOutcome uniformly.
- Delete old mixed `AnalyzedStatement`, duplicate category/type/name classifiers, and direct syntax→ExecutionPlan lowering.
- Update plan cache to store semantic/logical artifacts with all profile/catalog/session dependencies.
- Update conformance evidence and golden diagnostics.

### Non-goals

- No physical planner/batch executor; the existing executor remains behind a logical→legacy physical bridge.
- No localization.
- No broad rewrite of every error message beyond structured correctness.

### Acceptance evidence

- All request compilation failures return structured RequestOutcome diagnostics with expected statuses and source spans.
- Repository search finds no direct syntax→old plan or duplicate analyzed-statement path.
- Cache invalidation covers all recorded semantic dependencies and same-name object recreation.
- Golden diagnostics assert status/fields/spans rather than brittle full prose where possible.
- Logical plan is the sole semantic input to physical planning/legacy bridge.
- M05 milestone exit criteria and evidence records are green.

### Tests and gates

- Full positive/negative GQL corpus.
- Diagnostic precedence/additional/nested status tests.
- Plan-cache invalidation suite.
- Compiler stage API unit tests.
- Mutation tests for status mappings and cache checks.
- Parser fuzz and analyzer deep-recursion/cancellation regressions.

### Review focus

- Exact status mapping and uniform RequestOutcome.
- Old mixed path deletion.
- Cache dependency completeness.
- No physical planning mixed back into semantics.

### Stop conditions

- A status mapping cannot be verified from the selected profile/standard.
- Deleting old path would strand a statement family without logical lowering; split/finish that family first.
- Cache dependency model is incomplete.

### Bridge and deletion

- Retain only logical→legacy physical/executor bridge, owned by M06-PR07.
- All syntax/analyzer compatibility adapters are deleted.

<a id="m06-pr01"></a>
## M06-PR01 — Define Physical Plan Nodes, Properties, and Operator Contracts

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M05-PR06, M04-PR02
- **Issues:** None
- **Commit scope:** `executor`

Introduce a physical plan separate from logical algebra, with explicit algorithms, required/provided properties, resource estimates, and a pull-based batch operator interface.

### Scope

- Define immutable physical plan/operator descriptors for scans, filters, projections, joins, aggregates, sorting, paging, path, procedures, mutations, DDL, and transaction/session commands.
- Define required/provided properties: schema, order, duplicate semantics, partitioning/single-threaded status, graph snapshot, rewindability, cardinality estimate, memory budget, and effect class.
- Define `Operator::open`, `next_batch`, `close` or an equivalent lifecycle with cancellation/error/outcome context.
- Implement logical→physical planning for unit/empty and placeholder operators with validation and deterministic debug snapshots.
- Separate rule/cost decisions from operator runtime state and make physical plans cacheable only where generation-safe.
- Add plan validator and operator factory/registry internal APIs.

### Non-goals

- No real batch storage/operators beyond unit/empty scaffolding.
- No async/parallel scheduler.
- No JIT.
- No public physical plan API stability promise.

### Acceptance evidence

- A trivial logical plan lowers, validates, instantiates, and executes through the new operator lifecycle.
- Physical plan debug output distinguishes algorithm choices from logical semantics.
- Validator rejects schema/property/lifecycle incompatibilities.
- Runtime state cannot be shared accidentally across concurrent requests.
- No physical node stores raw syntax/catalog names or public row IDs.
- Logical→legacy bridge remains available for unimplemented operators and is explicitly marked.

### Tests and gates

- Physical plan golden/validation fixtures.
- Operator lifecycle tests including early close, error, and cancellation.
- Runtime-state isolation tests.
- Logical property/enforcer tests.
- Mutation tests around validator/lifecycle branches.

### Review focus

- Semantic/physical separation.
- Lifecycle correctness and cancellation.
- Immutable plan versus per-request state.
- No storage-name leakage.

### Stop conditions

- Operator contract requires a final batch layout not yet designed; use an opaque `Batch` trait/type placeholder with exact semantics.
- Logical properties are insufficient to choose/enforce physical behavior.
- The bridge becomes the default without per-operator migration tracking.

### Bridge and deletion

- A `LegacySubplan` physical node may execute unsupported logical fragments temporarily.
- Delete all `LegacySubplan` use in M06-PR07.

<a id="m06-pr02"></a>
## M06-PR02 — Implement Typed Binding Batches, Column Vectors, and Active-Row Selection

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M06-PR01, M05-PR03
- **Issues:** None
- **Commit scope:** `executor`

Create the physical binding-table batch representation with typed columns, null/reference handling, preferred column order, and adaptive dense/selection-vector/bitmap active-row forms.

### Scope

- Define `BindingBatch`, `ColumnVector`, `BatchSchema`, `RowSelection`, batch builder, scalar accessor, and row materialization interfaces.
- Support selected value/reference/list/record/path families without losing declared descriptors or null semantics.
- Implement active-row representations: dense range, selection vector, and bitmap, with explicit conversions and validated cardinality.
- Define default/configurable batch capacity and maximum memory safeguards.
- Preserve unordered/ordered status and preferred/effective column sequence metadata.
- Provide temporary conversion to/from semantic immutable BindingTable and legacy row tables for differential tests.

### Non-goals

- No full operator suite.
- No compressed/dictionary/Arrow ABI promise.
- No public zero-copy interoperability contract.
- No GPU column representation.

### Acceptance evidence

- All selected value families round-trip semantic rows → batches → rows with identical descriptors, values, nulls, references, order, duplicates, and preferred columns.
- Dense/selection/bitmap forms produce identical active rows and fail on invalid indices/cardinality.
- Batch size/memory limits return typed resource errors without partial output.
- No public facade exposes column internals.
- Allocation/iteration benchmarks establish the representation decision for later operators.
- Legacy conversion is isolated and tagged for M06-PR07 deletion.

### Tests and gates

- Property tests for row/batch round trips across value/type families.
- Active-row representation equivalence tests.
- Invalid length/index/null/reference fixtures.
- Ordering/duplicate/preferred-column metadata tests.
- Memory-limit/cancellation tests.
- Mutation tests around selection conversion and null handling.

### Review focus

- Declared type/null/reference preservation.
- Selection representation correctness.
- Small-query allocation cost.
- No premature external ABI.

### Stop conditions

- A selected type cannot be represented without semantic loss.
- Batch memory model requires unsafe code; repository policy forbids it unless separately approved.
- One generic `Vec<Value>` path makes later performance goals impossible without a clear specialization plan.

### Bridge and deletion

- Row/table conversion exists only for tests/results/legacy bridge.
- Delete legacy row-executor conversions in M06-PR07 where no longer needed.

<a id="m06-pr03"></a>
## M06-PR03 — Implement Batch Scan, Match Seed, Filter, Project, and Page Operators

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M06-PR02, M04-PR02
- **Issues:** None
- **Commit scope:** `executor`

Move the streaming unary core to batches: graph/index scans produce typed candidates, expressions evaluate over active rows, and projection/paging preserve exact result metadata.

### Scope

- Implement unit, empty, node/edge label/all-element, typed property point/range/prefix, and candidate-set scans against immutable graph snapshots.
- Implement vectorized/batch expression evaluation sufficient for predicates and projection over selected scalar/reference/property expressions.
- Implement filter with three-valued Boolean semantics and adaptive row selection.
- Implement project/return/select aliases, record construction, column rename/order, distinct where locally appropriate, offset, limit, and page streaming.
- Add physical planner rules for scan/index selection using catalog index descriptors without leaking rows.
- Differential-test every migrated fragment against the legacy executor and simple row reference.

### Non-goals

- No joins/group/sort/path quantification.
- No mutation/DDL/procedure execution.
- No broad cost optimizer rewrite.
- No expression JIT/SIMD requirement beyond evidence-backed primitives.

### Acceptance evidence

- Unary query corpus produces result/status/type/order/duplicate parity with the semantic reference.
- Index and full scan plans return identical results across compaction/snapshot generations.
- Filter null/unknown/type-error behavior and source diagnostics are exact.
- LIMIT/OFFSET avoid reading unnecessary upstream batches in instrumentation tests.
- One-row and small-result latency stays within the reviewed alpha budget; medium scans show reduced interpretation/allocation overhead.
- No row-space type crosses operator/public boundaries.

### Tests and gates

- Differential unary query corpus.
- Expression truth-table/type/error tests.
- Index/full-scan equivalence property tests.
- Early-stop/operator-close tests.
- Compaction/generation tests.
- Mutation tests for active-row/filter/paging branches.

### Review focus

- Three-valued filter correctness.
- Snapshot/candidate generation safety.
- Early stop and resource cleanup.
- Benchmark evidence includes one-row and scan workloads.

### Stop conditions

- Expression family migration balloons beyond PR cap; split by type family while keeping operator acceptance explicit.
- Index descriptor/catalog integration is not ready.
- Batch results differ in order/duplicates/type metadata.

### Bridge and deletion

- Unsupported expressions/subplans may use explicit legacy expression/subplan adapters.
- All unary bridge use is removed by M06-PR07.

<a id="m06-pr04"></a>
## M06-PR04 — Implement Batch Joins, Grouping, Aggregation, Sort, and Set Operations

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M06-PR03
- **Issues:** None
- **Commit scope:** `executor`

Move the pipeline breakers and multi-input operators to typed batches with correct duplicate, null, ordering, natural-join, and resource semantics.

### Scope

- Implement cross product, natural/hash join, optional/left semantics where selected, and join-key equality/null behavior.
- Implement group key construction, aggregate state/finalization, DISTINCT aggregates, empty-input semantics, and group variable/list handling for selected features.
- Implement stable/deterministic sort according to type/collation/null ordering rules and page/limit integration.
- Implement union/all, deduplication, distinct, and compatible schema/type combination.
- Add bounded memory accounting, spill-not-supported diagnostics, cancellation, and physical algorithm selection.
- Differential-test against row/reference execution and pure small-table models.

### Non-goals

- No disk spill or distributed partitioning in 2.0 initial scope.
- No worst-case-optimal joins unless an evidence-backed later optimization.
- No path joins beyond opaque path values.
- No code generation.

### Acceptance evidence

- Join/group/aggregate/sort/set corpus matches reference results including duplicates, nulls, types, statuses, and ordering.
- Empty/unit/all-null and incompatible-type edge cases are explicit.
- Memory budget and cancellation tests leave no leaked state and return structured errors.
- Hash keys respect decimal/string/collation/reference identity and selected comparability rules.
- Small-query latency and large-input throughput are both recorded before choosing defaults.
- No operator silently spills or changes order semantics.

### Tests and gates

- Pure small-table model/property tests for join/set/group.
- Aggregate truth/overflow/empty/null tests.
- Collation/order tests tied to profile.
- Memory/cancellation/failure cleanup tests.
- Differential corpus and mutation tests.
- Plan property/enforcer tests.

### Review focus

- Value semantics in keys.
- Ordering/duplicate/null correctness.
- Memory failure behavior.
- Algorithm choice evidence.

### Stop conditions

- Required value semantics from M05 are incomplete.
- Memory budget cannot be enforced before allocation.
- Reference and batch executor disagree on unordered result ordering tests; tests must compare as multisets where semantics allow.

### Bridge and deletion

- Legacy multi-input operators may remain only for unsupported features and are enumerated.
- Delete bridge in M06-PR07.

<a id="m06-pr05"></a>
## M06-PR05 — Implement Batch Mutation, Catalog, Session, and Transaction Operators

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M06-PR04, M03-PR04, M08-PR01
- **Issues:** None
- **Commit scope:** `executor`

Execute all side-effecting logical operations through batch-aware sinks and the M03 transaction state machine without bypassing validation, catalog ownership, or durability abstraction.

### Scope

- Implement insert, set, remove, delete, finish, catalog DDL, session commands, and transaction command physical operators for selected profile forms.
- Stage row batches into typed graph/catalog transaction deltas using stable IDs and semantic write sets.
- Materialize defaults, validate types/graph types/constraints, and compute before/after values through one mutation funnel.
- Guarantee statement atomicity inside the transaction on operator/type/runtime failure.
- Return omitted/row results and diagnostics exactly according to logical outcomes.
- Remove executor paths that call `SharedGraph::begin_write` or catalog publication directly.

### Non-goals

- No final WAL/durable commit implementation.
- No parallel writes or MVCC.
- No new DDL features.
- No native procedure adapters yet.

### Acceptance evidence

- Write/DDL/session/transaction corpus matches reference state and statuses.
- Failure in any input batch leaves no partial statement publication or staged leakage.
- Successor statements in an explicit transaction see staged prior writes.
- No direct publication/write-lock acquisition remains in GQL operators.
- Defaults/type/constraint validation uses canonical services and stable IDs.
- Auto-commit and explicit commit share the same physical transaction path.

### Tests and gates

- Batch mutation integration/property tests.
- Multi-batch atomicity/failure injection.
- Explicit/implicit transaction scenarios.
- Defaults/type/constraint and graph-type validation tests.
- Catalog/data mixing and read-only diagnostics.
- Mutation tests around rollback paths.

### Review focus

- No per-batch publication.
- Statement/transaction atomicity.
- One mutation/validation funnel.
- No direct graph-root writes.

### Stop conditions

- Constraint/index catalog foundation is not ready; reorder rather than duplicate.
- Current defaults/schema validation cannot operate on transaction-visible state.
- A side-effecting statement remains only in the legacy executor.

### Bridge and deletion

- In-memory commit authority remains until M09.
- Legacy side-effect executor paths must be deleted in this PR for migrated forms.

<a id="m06-pr06"></a>
## M06-PR06 — Implement Batch Procedure Calls and Native Value/Result Adapters

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M06-PR05, M05-PR04
- **Issues:** None
- **Commit scope:** `procedure`

Run query/data/catalog native procedures through typed descriptors, batch arguments, side-effect classes, and structured outcomes without row-executor or concrete graph leakage.

### Scope

- Define internal typed procedure descriptor/call/result interfaces compatible with catalog IDs, parameter descriptors, result binding-table types, and effect classes.
- Adapt current `BuiltinProcedureRegistry` and `algo.*` procedures to batch arguments/results and database-owned graph handles.
- Validate mandatory/optional/default arguments, named/positional rules, yield fields, result descriptors, and purity before execution.
- Support read-only query procedures and staged data/catalog procedures through transaction context.
- Convert native vector/JSON/list/record/path/reference values without stringification or row tables.
- Instrument procedure calls for cancellation, resource use, diagnostics, and stable procedure generation dependencies.

### Non-goals

- No final procedure catalog/extension documentation; M10-PR04.
- No loadable extensions.
- No new native features.
- No async streaming procedure ABI.

### Acceptance evidence

- All current built-ins compile through the adapter or are explicitly deferred with no stable facade exposure.
- Argument/yield/result type errors return exact diagnostics before side effects.
- Query procedures cannot acquire write capabilities; data/catalog procedures stage through transaction.
- Batch and legacy/reference results match for migrated procedures.
- No production procedure signature accepts concrete graph row/index/store types.
- Cancellation/resource accounting works across procedure execution.

### Tests and gates

- Descriptor/argument/default/yield validation tests.
- Purity/capability compile/runtime tests.
- Differential current procedure corpus.
- Batch result type/order/duplicate tests.
- Cancellation/failure cleanup tests.
- Mutation tests for descriptor dispatch.

### Review focus

- Capability-based procedure context.
- Descriptor/result correctness.
- No loadable ABI creep.
- Stable generation/cache behavior.

### Stop conditions

- A procedure requires direct mutable graph access not expressible through transaction capabilities; redesign/split it.
- Native value types cannot batch round-trip.
- Adapter would expose lower-crate concrete types through facade.

### Bridge and deletion

- Builtin registry adapter remains until M10-PR04 catalog cutover.
- Legacy row procedure execution is deleted for migrated procedures.

<a id="m06-pr07"></a>
## M06-PR07 — Cut Over to the Batch Executor and Delete the Row-at-a-Time Bridge

- **Owner:** M06
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M06-PR06
- **Issues:** None
- **Commit scope:** `executor`

Make physical batch execution the sole production runtime, close remaining operator gaps or mark features unsupported, and delete legacy subplan/row executor paths.

### Scope

- Inventory every remaining `LegacySubplan`, row operator, row expression, row procedure, and direct StatementOutput path.
- Implement missing selected-profile operators or explicitly downgrade unsupported feature states before deletion.
- Route `Session::execute` solely through syntax→semantic→logical→physical→batch execution.
- Delete legacy execution plan, row-at-a-time runtime, adapters, duplicate expression/category/error logic, and bridge benchmarks.
- Establish the 2.0 batch benchmark guard set and per-section baseline SHA/date.
- Run full differential, fuzz, mutation, and resource/cancellation validation appropriate to the executor cutover.

### Non-goals

- No path automata beyond currently supported bridge-free subset; M07 expands it.
- No JIT.
- No persistence cutover.
- No performance tuning unrelated to demonstrated regressions.

### Acceptance evidence

- Repository search finds no production legacy subplan/row executor/old ExecutionPlan path.
- All claimed/supported non-path statement families execute through batch operators and complete conformance evidence.
- Full differential corpus is green; unsupported forms fail deliberately and truthfully.
- One-row latency, scan/join/group throughput, memory, and cancellation results are recorded and reviewed.
- Public facade output and diagnostics remain stable across cutover.
- M06 milestone exit gate and full workspace/release-relevant tests pass.

### Tests and gates

- Full workspace nextest/doctests and GQL corpus.
- Differential semantic reference versus batch engine.
- Fuzz parser plus plan/operator construction inputs where available.
- Mutation testing across physical planner/expression/operators.
- Memory/cancellation/failure-injection suite.
- Public API and stale-bridge search checks.

### Review focus

- No hidden fallback remains.
- Truthful feature downgrade for gaps.
- Differential correctness before speed.
- Balanced latency/throughput/memory evidence.

### Stop conditions

- A claimed feature still depends on legacy execution.
- Differential mismatch is unexplained.
- Performance regression is large and the bridge deletion would remove the oracle before resolution.
- PR exceeds size cap because too many feature gaps remain; split explicit prerequisite PRs.

### Bridge and deletion

- Delete all production legacy bridges.
- Test-only reference row evaluators may remain clearly separated and non-exported.

<a id="m07-pr01"></a>
## M07-PR01 — Define Path Pattern Semantic IR and Automata Lowering

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M05-PR06, M04-PR04
- **Issues:** None
- **Commit scope:** `path`

Translate GQL path pattern expressions into an explicit semantic automaton with bindings, label/property predicates, quantifiers, origins, and finite-result obligations.

### Scope

- Define normalized semantic nodes for node/edge element patterns, concatenation, alternation, multiset alternation, grouping, quantified primaries, questioned primaries, variables, yields, where clauses, path modes, match modes, and selective prefixes.
- Lower path expressions to an epsilon-NFA or equivalent automaton with transition predicates and binding actions while preserving source/semantic origins.
- Model anonymous/temporary variables, singleton/group degree, parenthesized/subpath markers, and output exposure explicitly.
- Analyze unbounded quantifiers and record the exact finite-result guard required from restrictive mode, different-edges mode, or selective search.
- Define deterministic automaton normalization/validation and debug snapshots.
- Keep physical graph traversal out of this PR.

### Non-goals

- No product-graph execution.
- No shortest-path algorithm.
- No compact path storage.
- No parser grammar expansion beyond fixes needed to represent already selected syntax.

### Acceptance evidence

- Representative and exhaustive small path syntax lowers to deterministic valid automata with origin mappings.
- Questioned versus quantified variables and set versus multiset alternation remain distinguishable in IR/tests.
- Unbounded patterns lacking required guards fail annotation with expected feature/status diagnostics.
- Automaton validation catches unreachable/invalid state, binding, type, and orientation inconsistencies.
- No graph row/index/traversal algorithm type appears in semantic automata.
- A simple reference matcher can consume the automaton in tests.

### Tests and gates

- Golden semantic path/automaton snapshots.
- Property tests comparing regex-style syntax acceptance with a simple reference language generator for bounded expressions.
- Finite-result guard negative tests.
- Variable degree/exposure tests.
- Mixed edge orientation transition tests.
- Parser fuzz and automaton validator mutation tests.

### Review focus

- Semantic fidelity of lowering.
- Finite-result obligations.
- Variable degree/exposure.
- No physical traversal leakage.

### Stop conditions

- A syntax family cannot be normalized without losing GQL semantics.
- Finite-result rules cannot be verified for a selected feature.
- Automaton representation conflates set/multiset or path orientation.

### Bridge and deletion

- Current path syntax/planner code may be used only to cross-check snapshots.
- Production execution moves to automaton operators by M07-PR06.

<a id="m07-pr02"></a>
## M07-PR02 — Implement Product-Graph Execution for Bounded Paths

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M07-PR01, M06-PR02
- **Issues:** None
- **Commit scope:** `path`

Execute bounded path automata over immutable mixed-graph snapshots using a reference-correct product-graph traversal that emits path bindings and binding batches.

### Scope

- Define product states over graph node/reference, automaton state, accumulated binding state, and bounded quantifier counters.
- Implement bounded BFS/DFS or worklist traversal with deterministic reference enumeration for node/edge transitions and mixed orientation.
- Evaluate label expressions at transition time and search conditions at the semantically correct stage; permit safe predicate pushdown only with equivalence tests.
- Emit reduced path/element bindings into binding batches and naturally join with incoming working records/tables.
- Support anchored/unanchored endpoints and repeated declarations for the bounded selected subset.
- Add cancellation, visit/result/memory budgets that report resource limits without altering valid finite semantics under configured supported bounds.

### Non-goals

- No unbounded restrictive modes or selective shortest searches.
- No compact PMR backend.
- No cost-based specialization.
- No parallel traversal.

### Acceptance evidence

- Bounded path fixtures across directed/undirected/mixed multigraphs match exhaustive enumeration.
- Anchored/unanchored and repeated-variable natural joins are correct.
- Label/predicate stage and pushdown equivalence have focused tests.
- Cancellation/resource failures do not emit partial successful results as complete outcomes.
- Path bindings integrate with batch joins/projections and preserve declared schemas.
- No duplicate edge identity is created for undirected traversal.

### Tests and gates

- Exhaustive small graph/pattern differential tests.
- Mixed orientation/self-loop/parallel-edge fixtures.
- Binding/natural-join/set/multiset tests.
- Predicate pushdown differential tests.
- Cancellation/resource/failure cleanup tests.
- Mutation tests for product-state dedup and transition filtering.

### Review focus

- Product-state identity and dedup correctness.
- Predicate evaluation stage.
- Mixed-edge orientation.
- Resource errors versus semantic truncation.

### Stop conditions

- Traversal budgets would silently truncate a claimed valid result instead of returning an error.
- Binding state makes product-state identity unbounded beyond the bounded pattern model.
- Batch join integration changes path match multiplicities.

### Bridge and deletion

- Current bounded matcher may remain test-only as an oracle until M07-PR06.
- No production fallback for migrated bounded patterns after this PR.

<a id="m07-pr03"></a>
## M07-PR03 — Implement WALK, TRAIL, SIMPLE, ACYCLIC, and Match Modes

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M07-PR02
- **Issues:** None
- **Commit scope:** `path`

Extend product execution with exact path/match restrictions and finite-state tracking for repeated edges/nodes across directed, undirected, and mixed graphs.

### Scope

- Implement WALK with no mode restriction, TRAIL with no repeated edge, ACYCLIC with no repeated node, and SIMPLE with only optional first/last node repetition.
- Implement DIFFERENT EDGES across edge variables and REPEATABLE ELEMENTS behavior at graph-pattern scope.
- Support mode scopes on parenthesized expressions and complete path patterns, including nested mode state.
- Use persistent/compact visitation state keyed by stable IDs and occurrence scope, never by rows across generations.
- Handle loops, undirected reversal, repeated variables, alternation, and quantifiers explicitly.
- Advance feature/evidence states only for fully covered modes.

### Non-goals

- No selective shortest prefixes.
- No arbitrary heuristic pruning.
- No claim that SIMPLE implies TRAIL.
- No parallelization.

### Acceptance evidence

- Exhaustive small-graph tests match a simple path enumerator for every mode/match combination.
- Counterexamples prove SIMPLE does not imply TRAIL on undirected graphs.
- Self-loops, parallel edges, and reverse undirected traversals have correct repetition behavior.
- Nested mode scopes and DIFFERENT EDGES across variables are exact.
- Unbounded patterns under restrictive modes terminate with complete finite results for supported limits.
- Default match mode behavior is generated/profile-backed.

### Tests and gates

- Exhaustive mode/match differential matrix.
- Known counterexample fixtures.
- Nested scope and quantified variable tests.
- Unbounded restrictive-mode termination tests on cyclic graphs.
- Default profile mode tests.
- Mutation tests around visitation-state checks.

### Review focus

- Stable identity repetition semantics.
- Nested scope handling.
- No implication between modes.
- Finite complete results.

### Stop conditions

- Mode scope semantics are uncertain for a syntax form.
- Visitation state causes uncontrolled memory without a typed resource error.
- Reference enumerator disagrees on any small-graph case.

### Bridge and deletion

- Remove current ad hoc mode logic for migrated patterns.
- Keep only test reference enumerator.

<a id="m07-pr04"></a>
## M07-PR04 — Implement Selective Path Search Prefixes and Endpoint Partitioning

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M07-PR03
- **Issues:** None
- **Commit scope:** `path`

Add ANY, ALL SHORTEST, counted shortest, and counted shortest-group selection over endpoint-partitioned path bindings with deterministic nondeterminism policy and finite results.

### Scope

- Partition candidate path bindings by first and last node identity as required before selective choice.
- Implement ALL SHORTEST via multi-source/product-state distance discovery and complete minimal-path enumeration per partition.
- Implement counted shortest and counted shortest-group with the profile-defined deterministic choice/tie policy where the standard permits nondeterminism.
- Implement ANY counted selection with an explicit implementation-dependent selection policy and disclosure.
- Integrate search-condition evaluation at the required pre-selection stage.
- Add resource-aware shortest predecessor/path enumeration without silently dropping valid ties.

### Non-goals

- No weighted/cost shortest path extension.
- No global shortest across all endpoints.
- No approximate top-k.
- No distributed traversal.

### Acceptance evidence

- Selective results match exhaustive enumerate-filter-partition-select reference on small graphs.
- Fixtures prove per-partition shortest paths can have different lengths and are all retained correctly.
- Search conditions eliminate paths before final selective choice where required.
- Count zero/one/large, ties, unreachable endpoints, loops, and mixed edges are covered.
- Resource exhaustion returns a failed outcome and never presents a partial tie set as complete.
- Implementation-dependent choice policy is disclosed/generated.

### Tests and gates

- Exhaustive selective reference differential tests.
- Endpoint partition/tie/group fixtures.
- Predicate stage tests.
- Count boundary/type/error tests.
- Resource/cancellation/failure tests.
- Mutation tests for distance/tie/partition logic.

### Review focus

- Partitioning and tie completeness.
- Product-state distance.
- Predicate timing.
- No partial-success truncation.

### Stop conditions

- A selective rule cannot be verified.
- Predecessor/path representation cannot enumerate all required ties within resource model without a clear error.
- Implementation-dependent policy is undocumented.

### Bridge and deletion

- No legacy selective execution remains after this PR for supported forms.
- Weighted/native shortest procedures remain separate extensions.

<a id="m07-pr05"></a>
## M07-PR05 — Implement Path Values and Compact Shared Path Representation

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M07-PR04, M05-PR03
- **Issues:** None
- **Commit scope:** `path`

Make path values first-class typed results while introducing an evidence-gated shared-prefix/DAG representation that avoids explosive intermediate copying without changing enumeration semantics.

### Scope

- Define `PathValue` descriptor/value over ordered node/edge reference occurrences with traversal orientation metadata and validated alternation/connectivity.
- Implement a request-local `PathArena`/shared predecessor DAG that reuses prefixes/suffixes for intermediate path bindings.
- Provide deterministic lazy/materialized enumeration, equality/identity semantics, length/element access, projection, and facade result conversion.
- Ensure path variables, list/group bindings, and compact representation preserve duplicates and selection order where semantically relevant.
- Add thresholds/strategy based on measured memory/time; retain a simple materialized reference representation in tests.
- Document research basis and explicitly avoid claiming a full academic PMR implementation unless actually achieved.

### Non-goals

- No persisted path-value storage unless selected property profile later requires it.
- No public PathArena ABI.
- No random sampling/counting extensions.
- No lossy compression or approximate enumeration.

### Acceptance evidence

- Path values round-trip/materialize with exact nodes, edges, orientations, length, and reference validity.
- Shared representation and simple materialized reference produce identical ordered/multiset results across the path corpus.
- High-prefix-sharing fixtures demonstrate reviewed memory reduction without unacceptable tiny-path latency regression.
- Invalid connectivity/alternation/reference/generation cases are rejected.
- Facade results own/materialize data safely before request teardown.
- No user-visible enumeration changes when strategy threshold changes.

### Tests and gates

- PathValue construction/access/equality/reference tests.
- Arena/reference differential property tests.
- Request lifetime/materialization tests.
- High-sharing and no-sharing fixtures.
- Cancellation/resource tests.
- Mutation tests around arena linking/enumeration.

### Review focus

- Semantic invisibility of compact representation.
- Request lifetime and facade ownership.
- Orientation/connectivity validation.
- Balanced tiny/large evidence.

### Stop conditions

- Compact representation changes result order/multiplicity.
- Lazy values escape request lifetime.
- Memory savings are not demonstrated and complexity is not justified; retain simple representation.

### Bridge and deletion

- Simple materialized implementation remains test oracle.
- No production duplicate path representation after strategy decision.

<a id="m07-pr06"></a>
## M07-PR06 — Integrate Path Planning, Differential Conformance, and Delete Legacy Path Execution

- **Owner:** M07
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M07-PR05
- **Issues:** None
- **Commit scope:** `path`

Make automaton/product-graph execution the sole production path engine, add physical strategy selection and complete the selected path evidence corpus.

### Scope

- Add physical path strategies for anchored/unanchored, bounded/restrictive, shortest/selective, predicate pushdown, and compact-result choices.
- Implement cost/cardinality inputs and conservative fallback to the reference-correct product executor.
- Expand positive/negative/status/differential corpus across selected path grammar, modes, match modes, variables, predicates, and selective prefixes.
- Run exhaustive small-graph generation and targeted larger graph benchmarks.
- Delete legacy path matcher/traversal planner and any production fallback.
- Update feature/evidence/claim status truthfully for completed versus deferred path features.

### Non-goals

- No unsupported optional path syntax added to inflate coverage.
- No weighted path extension.
- No parallel/distributed engine.
- No removal of test-only reference enumerators.

### Acceptance evidence

- Repository search finds no production legacy path matcher or old traversal fallback.
- Selected path feature corpus/evidence is complete and implication-closed.
- Exhaustive small-graph differential suite is green for all supported modes/prefixes.
- Optimized physical strategies match the reference and have recorded crossover evidence.
- Resource/cancellation and diagnostic behavior is consistent through facade outcomes.
- M07 milestone exit gate passes with truthful unsupported list.

### Tests and gates

- Full path corpus and generated small-graph differential suite.
- Physical strategy forcing tests.
- Parser/path fuzz and mutation tests.
- Resource/cancellation/failure injection.
- Conformance claim/evidence gate.
- Stale legacy bridge search.

### Review focus

- No legacy fallback.
- Reference equivalence of every optimization.
- Truthful feature scope.
- Resource behavior and diagnostics.

### Stop conditions

- Any optimized/reference mismatch remains unexplained.
- A claimed path feature lacks complete evidence.
- Performance work changes semantics or deletes the reference oracle.
- The PR includes multiple unplanned optional features.

### Bridge and deletion

- Delete all production legacy path bridges.
- Keep test-only exhaustive/reference evaluators as permanent correctness assets.

<a id="m08-pr01"></a>
## M08-PR01 — Create Catalog-Owned Index and Constraint Descriptors

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M02-PR03, M05-PR04, M04-PR05
- **Issues:** None
- **Commit scope:** `catalog`

Establish one lifecycle and metadata model for indexes and integrity constraints, including targets, backing relationships, build state, ownership, and transaction visibility.

### Scope

- Define stable `IndexId`/`ConstraintId`, names, graph/type ownership, element kind, target expressions/properties, index kind, uniqueness/constraint semantics, lifecycle/build state, backing links, generation, and extension/profile metadata.
- Move index registrations and unique/property annotations out of ad hoc graph maps/booleans into catalog descriptors or explicit adapters.
- Define create/drop/build/ready/failed/rebuild states and transaction-visible publication rules.
- Define constraint-to-backing-index requirements, ownership, automatic index creation naming, and shared-index compatibility.
- Expose Rust/catalog introspection APIs and semantic DDL command objects; grammar-specific extension syntax can remain in later PRs.
- Define derived-state provider interfaces keyed by catalog descriptors and graph generations.

### Non-goals

- No composite uniqueness enforcement yet.
- No expression index evaluation.
- No final persistence.
- No online concurrent index build outside the serial writer model.

### Acceptance evidence

- Indexes/constraints can be created, resolved, listed, inspected, and dropped through catalog APIs with dependency checks.
- Descriptor state transitions are validated and impossible transitions fail.
- A provider is bound to descriptor/graph/generation and reports ready/complete status.
- Existing property/composite/vector/text index registrations have an explicit migration mapping or scheduled native reintegration path.
- Dropping a backing index under an active constraint is rejected or cascades only through explicitly selected extension semantics.
- No new integrity metadata is stored as a property-level Boolean.

### Tests and gates

- Descriptor/lifecycle state-machine property tests.
- Dependency/share/drop tests.
- Provider capability/completeness tests.
- Catalog snapshot/generation tests.
- Migration adapter tests for existing indexes.
- Mutation tests around lifecycle and dependency checks.

### Review focus

- Metadata authority versus derived state.
- Constraint activation safety.
- Lifecycle/dependency state machine.
- Migration path without new booleans.

### Stop conditions

- Existing provider APIs cannot report completeness/graph generation.
- Descriptor target model is too narrow for M08-PR04 expression indexes.
- Catalog/data publication cannot atomically activate constraints.

### Bridge and deletion

- Adapters may expose current property/composite indexes as descriptor-backed providers.
- Delete adapters as each provider family migrates; core scalar/composite by M08-PR03, native by M10.

<a id="m08-pr02"></a>
## M08-PR02 — Implement Named Composite Unique and Key Constraints

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M08-PR01, M05-PR03
- **Issues:** #1092
- **Commit scope:** `constraint`

Close #1092 by making single- and multi-property/expression tuple constraints first-class type/catalog objects with typed canonical tuple encoding and explicit null/missing semantics.

### Scope

- Define ordered key component descriptors over properties initially, extensible to deterministic index expressions from M08-PR04.
- Implement named node/edge unique constraints and, where selected, key constraints with explicit label/type/domain ownership.
- Generalize current single-property `UNIQUE` to an arity-one constraint and remove `PropertyTypeDef::unique` as authoritative state.
- Define canonical typed tuple key encoding reusing value variant tags/normalization and preventing separator/stringification ambiguity.
- Lock semantics for missing/null components: unique constraints skip tuples with any null/missing component; key constraints require every component present and material (unless profile explicitly chooses otherwise).
- Add vendor/profile-gated DDL syntax for type-level constraints and descriptor introspection.

### Non-goals

- No index-backed incremental enforcement yet.
- No arbitrary cross-element/cross-graph constraint.
- No foreign keys.
- No compatibility for property annotation syntax unless retained as syntax sugar lowering to arity-one descriptor.

### Acceptance evidence

- Arity 1–N constraints are declarable and introspectable for node and edge types.
- Tuple encoding distinguishes type variants, embedded delimiters, null/missing, decimal normal forms, and component boundaries.
- Single-property behavior is exactly the arity-one case and no duplicate enforcement path remains.
- Null/missing semantics have explicit GQL/Rust tests and documentation.
- Whole-state activation detects duplicates and reports the conflicting constraint/domain with structured diagnostics.
- Issue #1092 workaround cases are covered and no derived concatenated key is required.

### Tests and gates

- Tuple codec property/fuzz tests across all supported key value types.
- Arity/node/edge/null/missing/duplicate whole-state validation tests.
- DDL parser/analyzer/descriptor snapshots.
- Property annotation desugaring parity tests if retained.
- Mutation tests for tuple component/order/null branches.
- Conformance/extension evidence updates.

### Review focus

- Tuple encoding and equality semantics.
- Null/missing policy.
- No dual single/composite paths.
- Catalog/type ownership.

### Stop conditions

- Key constraint semantics conflict with selected GQL graph-type key behavior and cannot be cleanly separated as extension.
- Tuple equality does not match query/index equality.
- DDL syntax cannot be kept clearly extension-profile gated.

### Bridge and deletion

- Current `unique: bool` may parse as sugar only during this PR.
- Delete old UniquePropertyKey/validator authority in M08-PR03.

<a id="m08-pr03"></a>
## M08-PR03 — Make Uniqueness Incremental and Backing-Index Enforced

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M08-PR02, M06-PR05
- **Issues:** #1094
- **Commit scope:** `constraint`

Close #1094 by activating constraints only with complete backing indexes and validating transaction deltas through transaction-visible probes rather than full alive-entity scans.

### Scope

- Bind every active unique/key constraint to a compatible complete scalar/composite index, automatically creating one when needed under deterministic catalog naming.
- Implement transaction-visible index overlays for staged create/update/delete so probes see base state plus current delta and exclude replaced/deleted rows.
- Validate only impacted constraint tuples and detect intra-transaction duplicate candidates before commit.
- Retain full-state validation only for activation/rebuild/recovery verification and remove full alive-node/edge scans from ordinary commits.
- Integrate constraint/index validation into one transaction preparation phase before durable append/publication.
- Delete old `type_validator/unique` per-property scan path and close #1094.

### Non-goals

- No lock-free concurrent writers; single writer remains.
- No deferred constraints.
- No partial/incomplete backing index accepted.
- No expression targets until M08-PR04.

### Acceptance evidence

- Ordinary unique/key writes perform O(delta × probe) validation and instrumentation proves no full alive scan.
- Create/update/delete/swap/cycle/bulk/intra-transaction duplicates are correct for nodes and edges.
- Constraint activation/rebuild catches existing duplicates before publication.
- Backing index incompleteness/failure produces a failed transaction, not a false success or hidden scan.
- Commit-path benchmarks eliminate the O(N²) bulk-load behavior from #1094.
- Old per-property unique validator and scan loop are removed.

### Tests and gates

- Transaction overlay property/model tests against full-state reference validator.
- Swap/update/delete/intra-transaction duplicate scenarios.
- Provider failure/incomplete/rebuild/activation tests.
- Instrumentation test asserting ordinary commits do not iterate all live entities.
- Recovery/compaction consistency adapter tests pending M09.
- Mutation tests for self-exclusion/overlay collapse.

### Review focus

- Transaction-visible overlay correctness.
- No silent scan fallback.
- Atomic activation/index relationship.
- Self/update and intra-transaction conflicts.

### Stop conditions

- Index provider cannot guarantee completeness at transaction generation.
- Overlay semantics are ambiguous for multiple writes to one entity.
- Ordinary path still scans alive sets in instrumentation.

### Bridge and deletion

- Full-state validator remains only for activation/rebuild/verification.
- Delete old per-property unique path completely.

<a id="m08-pr04"></a>
## M08-PR04 — Introduce Deterministic Scalar Expression Index Targets

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M08-PR01, M05-PR03, M05-PR04
- **Issues:** None
- **Commit scope:** `index`

Generalize index descriptors and maintenance from bare property lists to analyzed, canonical, deterministic scalar expressions over one node or edge.

### Scope

- Define canonical `IndexExpression` IR referencing one element variable/property and a whitelist of pure deterministic scalar operations/functions.
- Analyze/index-target expressions with exact type, nullability, collation, function version/profile, and dependency metadata.
- Reject parameters, session/request time, procedures, graph traversal, subqueries, aggregates, nondeterministic functions, external state, and cross-element references.
- Implement create/rebuild/maintenance evaluation over before/after element state and typed index dispatch.
- Implement semantic expression equivalence/fingerprinting for planner match, including normalized property-only/composite targets.
- Integrate expression target dependencies into schema/function/profile generation invalidation.

### Non-goals

- No arbitrary user function indexes.
- No multi-element/materialized-view indexes.
- No JSON-specific planner rule until M08-PR05.
- No persisted expression bytecode; persist canonical descriptor in M09.

### Acceptance evidence

- Property, simple cast/normalize, tuple component, and whitelisted function expressions create/rebuild/maintain correct indexes.
- Every forbidden nondeterministic/impure/dynamic form fails annotation with a precise reason.
- Expression fingerprint is deterministic and semantically stable across harmless source formatting/spelling normalization.
- Update/delete/recovery-model/compaction tests keep entries synchronized.
- Planner can match an equivalent analyzed predicate expression to the target descriptor.
- Function/profile/collation generation changes invalidate/rebuild affected indexes.

### Tests and gates

- Positive/negative index-expression analyzer corpus.
- Fingerprint normalization/property tests.
- Maintenance before/after model tests.
- Planner equivalence tests.
- Function/profile generation invalidation tests.
- Fuzz canonical expression decoder once persisted later.

### Review focus

- Determinism/purity proof.
- Semantic fingerprinting.
- Maintenance transactionality.
- Function/collation version dependencies.

### Stop conditions

- An expression cannot prove deterministic from canonical metadata.
- Fingerprint equivalence risks treating semantically different expressions as equal.
- Property-only performance regresses materially without specialization.

### Bridge and deletion

- Bare property/composite index descriptors lower to expression targets.
- Delete duplicate property-list maintenance code once parity is proven.

<a id="m08-pr05"></a>
## M08-PR05 — Add JSON Scalar-Path Expression Indexes and Planner Pushdown

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M08-PR04
- **Issues:** #1097
- **Commit scope:** `json`

Close #1097 by indexing deterministic JSON scalar paths through existing typed scalar providers and routing equivalent predicates to those indexes.

### Scope

- Whitelist canonical JSON scalar path extraction forms such as text/scalar extraction with literal path components over one JSON property.
- Infer a scalar index kind/type from the extraction/cast and reject object/array/non-scalar targets unless explicitly cast to a supported scalar.
- Maintain path-derived scalar entries on create/update/delete, including missing/null/type-change and malformed-selector behavior.
- Recognize semantically equivalent `json_get_path_text/value(...)=literal`, IN/range where type permits, and related predicate forms during planning.
- Retain exact JSON scan procedures as correctness/small-corpus fallback when no usable index exists, with explain/debug plan visibility.
- Add DDL/reference examples and close #1097; defer GIN-style containment.

### Non-goals

- No general JSON containment/inverted index.
- No wildcard/recursive/dynamic path selectors in indexes.
- No query rewrite that changes missing/null/error semantics.
- No removal of exact JSON oracle.

### Acceptance evidence

- The issue example retrieves through a registered string path index and returns scan-equivalent results.
- Nested object/array indices, missing, JSON null, scalar type changes, escaped keys, and updates are covered.
- Planner refuses near-but-not-equivalent paths/functions/casts and falls back visibly.
- Recovery logical model and compaction/rebuild tests reproduce index state.
- Indexed lookup shows sublinear/point-probe behavior versus label scan at larger corpus sizes.
- Issue #1097 closes without a shadow top-level property workaround.

### Tests and gates

- JSON path evaluator/index maintenance model tests.
- Planner match/refusal corpus.
- Exact scan versus indexed differential/property tests.
- Update/delete/missing/null/type-change fixtures.
- DDL/analyzer diagnostics.
- Mutation tests for path/fingerprint/missing branches.

### Review focus

- Exact missing/null/type semantics.
- Shared evaluator and planner equivalence.
- No containment scope creep.
- Measured graduation path beyond scan.

### Stop conditions

- Existing JSON functions have inconsistent missing/null semantics across query and index evaluator.
- Planner cannot prove semantic equivalence.
- Index maintenance requires parsing arbitrary dynamic paths.

### Bridge and deletion

- Keep exact JSON scan as permanent oracle/fallback.
- No shadow property compatibility path.

<a id="m08-pr06"></a>
## M08-PR06 — Resolve Read-Hot Map Regressions and Install Balanced Performance Gates

- **Owner:** M08
- **State:** Unmerged
- **Risk / size:** High / M
- **Dependencies:** M04-PR02, M06-PR07
- **Issues:** #1137
- **Commit scope:** `performance`

Close #1137 by selecting collection/layout backends per access pattern using isolated A/B evidence and by making read and write guard rows mandatory for future storage changes.

### Scope

- Reproduce the #1118 boundary/read regressions on current toolchain and add missing edge-label/read rows.
- A/B candidate backends/layouts for `idx_label`, `idx_edge_label`, `node_id_to_row`, and any secondary layout/cache effect on typed property probes.
- Measure read, clone, create, update, delete, commit, memory, and mixed 60/40 workloads at identical fixtures/toolchain.
- Select and document per-map backend rather than one workspace-wide collection ideology; implement the chosen targeted changes.
- Add permanent balanced storage-layout guard rows and section-level baseline SHA/date/fixture metadata.
- Close #1137 with a recorded decision even if some regression is deliberately accepted for a quantified write/memory benefit.

### Non-goals

- No broad storage rewrite.
- No changing semantic identity/candidate APIs.
- No unmeasured micro-optimization.
- No pretending benchmark thresholds are portable guarantees.

### Acceptance evidence

- The original read regressions and edge-side analog are reproduced or discrepancy explained with current evidence.
- Chosen map/layout decisions include read and write A/B plus memory and weighted workload view.
- Guard set fails on deliberate reintroduction of the measured read regression beyond configured review threshold.
- BENCHMARKS/performance docs have per-section source SHA/date/fixture/toolchain.
- No semantic behavior/API changes are bundled.
- Issue #1137 closes with the final decision and evidence.

### Tests and gates

- Existing graph/store correctness/property tests.
- Benchmark script fixture/invocation tests.
- Guard threshold parser tests.
- Map backend equivalence tests.
- Public identity/candidate regression suite.

### Review focus

- Balanced A/B and unchanged fixtures.
- Per-map rather than blanket decision.
- No causal overclaim for secondary effect.
- Permanent guard provenance.

### Stop conditions

- Benchmark noise/hardware drift prevents a confident decision.
- A candidate backend violates immutable snapshot or safe-code invariants.
- The change expands into storage architecture work owned by another milestone.

### Bridge and deletion

- No bridge expected.
- Benchmark baselines become the M08 accepted 2.0 storage reference and are restamped only by reviewed changes.

<a id="m09-pr01"></a>
## M09-PR01 — Introduce the Anchored `StoreDirectory` Filesystem Capability

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M00-PR04
- **Issues:** #1088
- **Commit scope:** `persist`

Close #1088 by opening one directory authority and performing every managed artifact operation relative to that capability rather than re-resolving caller paths.

### Scope

- Define `StoreDirectory`/`DirectoryCapability` opened from a path or accepted as an already-open capability, with stable identity metadata and platform-specific backend abstraction.
- Implement managed relative open/create/rename/remove/fsync/list operations for WAL, manifest/current, locks, snapshots, archives, temp files, and audit files.
- Use Unix handle-relative operations and Windows directory-handle-equivalent semantics with final-component/no-follow checks as available.
- Make `DatabaseBuilder::open` acquire one capability and pass clones/borrows to every persistence component.
- Keep path APIs only as convenience constructors that immediately anchor and never re-resolve during the operation lifetime.
- Add deterministic rename-and-replace race hooks and close #1088.

### Non-goals

- No WAL format change.
- No sandbox/general filesystem capability library.
- No network/shared filesystem guarantee beyond documented platform semantics.
- No 1.x compatibility requirement for API signatures.

### Acceptance evidence

- Rename original directory and create replacement at old path between setup/open hooks; all artifacts touch only the anchored original.
- Replacement receives no WAL, MANIFEST/CURRENT, lock, snapshot, archive, temp, or audit artifact.
- Writer and manifest locks share one anchored identity/domain.
- Checkpoint/recovery/audit APIs require the same capability type or a controlled clone.
- Traversal, symlink/reparse, absolute-path, and invalid-component attempts fail safely.
- Issue #1088 closes with Unix and Windows evidence or an explicit supported-platform matrix.

### Tests and gates

- Deterministic rename/replacement TOCTOU tests.
- Symlink/reparse/traversal/final-component tests by platform.
- Capability clone/rename/identity tests.
- Parent/file fsync ordering test doubles.
- Miri/sanitizer not required but safe-code and handle lifecycle tests.
- Mutation tests around path validation.

### Review focus

- No pathname re-resolution after anchor.
- All artifact families migrate.
- Platform semantics and fallback honesty.
- Handle/lock/fsync lifetime.

### Stop conditions

- A supported platform cannot provide the required capability semantics without unsafe code or a reviewed dependency.
- Any artifact operation still accepts/resolves an independent path.
- Cross-platform behavior cannot be tested or explicitly scoped.

### Bridge and deletion

- Current path entry points may remain only as immediate `StoreDirectory::open(path)` wrappers.
- Delete independent path operations as each artifact module migrates.

<a id="m09-pr02"></a>
## M09-PR02 — Define the 2.0 Store Layout, Store Identity, Lock, Epoch, and Manifest Model

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR01, M04-PR05
- **Issues:** None
- **Commit scope:** `persist`

Specify and implement the directory-level control model that binds one database/store ID, format generation, lock domain, catalog/graph epoch, CURRENT pointer, and manifest lineage.

### Scope

- Define format 2 directory names and controlled artifact classes: LOCK, CURRENT, MANIFEST generations, WAL segments, snapshots, archives, temp/staging, audit.
- Define stable `StoreId`, format major/minor, store epoch, manifest generation, transaction sequence, checkpoint ID, and catalog/graph generation fields.
- Implement single-writer lock acquisition/identity and read/open policy through `StoreDirectory`.
- Define immutable manifest records and atomic CURRENT publication protocol with checksum and parent-directory durability.
- Define clean empty-store creation and open validation without yet writing transaction WAL/snapshots.
- Reject foreign/mixed store IDs, unsupported versions, stale CURRENT, and split lock domains with typed errors.

### Non-goals

- No WAL transaction encoding.
- No snapshot sections.
- No multi-process concurrent readers/writer guarantee beyond explicit lock policy.
- No 1.x migration.

### Acceptance evidence

- Create/open/reopen an empty format 2 store with stable IDs and manifest lineage.
- Mixed artifact StoreId/version/epoch/generation fixtures fail before any mutation.
- CURRENT/manifest atomic publication survives injected crashes at each file/sync/rename phase.
- Only one writer lock domain exists under the anchored directory; second writer fails deterministically.
- Manifest identifies profile/Unicode/collation/format versions required for safe reopen.
- No artifact is overwritten in place except controlled lock/temp semantics.

### Tests and gates

- Crash/failure matrix for empty-store create and CURRENT publication.
- Mixed/corrupt/stale artifact fixtures.
- Writer lock contention/identity tests.
- Store ID/epoch/version codec round trips.
- Directory capability rename integration.
- Fuzz manifest decoder after framing exists.

### Review focus

- Atomic CURRENT/manifest protocol.
- Store identity and mixed-file rejection.
- Lock domain.
- Profile/collation identity.

### Stop conditions

- Filesystem publication semantics cannot be made explicit on a supported platform.
- Manifest is asked to carry mutable hot state better suited to WAL/snapshot.
- A 1.x reader/migrator is proposed.

### Bridge and deletion

- No production 1.x layout bridge.
- Current manifest code may remain in archive/test fixtures until M09-PR08 deletion.

<a id="m09-pr03"></a>
## M09-PR03 — Implement WAL Segment Writer Watermarks and Precise Commit Outcomes

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR02, M03-PR04
- **Issues:** #1128
- **Commit scope:** `wal`

Close #1128 by implementing explicit written, flushed, published, and acknowledged positions and by making cancel/rollback versus indeterminate outcomes depend on durable phase evidence.

### Scope

- Define segment writer state with logical sequence plus byte offsets for initialized/header, written, flushed, published, acknowledged, and sealed positions where applicable.
- Implement append framing hooks, `flush/sync_data`, durable truncate/cancel back to flushed position, segment seal, and unusable/poisoned transitions.
- Define failure classification for validation/encode, append-before-write, partial write, flush, durable truncation, publish, acknowledge, and post-flush panic.
- Integrate the commit authority state machine so definitely unwritten/canceled work reports rollback/canceled while flushed-but-publication-unknown reports indeterminate.
- Address multiple durable providers by locking the 2.0 decision to one authoritative WAL; other providers cannot make commit-critical partial decisions.
- Invert/split current indeterminate tests and close #1128.

### Non-goals

- No final transaction payload schema; M09-PR04.
- No group-commit optimization beyond state-machine hooks.
- No replication.
- No 1.x WAL writer compatibility.

### Acceptance evidence

- Partial append/flush failures leave no recoverable trace of error-acked canceled transactions when durable truncation succeeds.
- Those transactions report rollback/canceled status, not 40003.
- A publish-tail panic after successful authoritative flush remains indeterminate and its record recovers.
- Watermark invariants and writer poison behavior are property/model tested across failure points.
- No second commit-critical provider can accept a transaction independently.
- Issue #1128 tests are updated and issue closes with phase-specific evidence.

### Tests and gates

- Deterministic short-write/partial-write/flush/truncate/fsync/publish/ack failure injection.
- State-machine model/property tests for watermarks.
- Reopen tests for each failure phase.
- Group/batch member phase classification tests.
- Writer poison/reuse/seal tests.
- Mutation tests around outcome classification.

### Review focus

- Truncation durability, not just `set_len`.
- Rollback versus indeterminate precision.
- One authoritative provider.
- Monotonic state invariants.

### Stop conditions

- A platform cannot durably truncate to flushed offset under the selected guarantees.
- Commit publication still occurs before flush.
- A derived provider remains commit-critical.
- Any failure case is classified optimistically without proof.

### Bridge and deletion

- The writer may initially append opaque test payload frames.
- Payload integration comes in M09-PR04; no 1.x writer path.

<a id="m09-pr04"></a>
## M09-PR04 — Define and Encode the Logical Transaction WAL Record Format 2.0

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR03, M04-PR05
- **Issues:** None
- **Commit scope:** `wal`

Encode the M04 logical transaction bundle into checksummed, length-delimited WAL records with stable versioning, limits, corruption detection, and replay validation.

### Scope

- Choose and document the format 2 record framing/codec based on deterministic encoding, safe bounded decode, forward minor-version skipping policy, and benchmark evidence.
- Encode transaction ID/sequence, store ID/epoch, base/result catalog and graph generations, logical changes, profile/function/collation dependencies, timestamp, and optional audit metadata.
- Add record and payload length limits, counts, nesting limits, checksums, segment/record versioning, and canonical ordering validation.
- Distinguish incomplete/torn final record from checksum/structural corruption in sealed or interior regions.
- Implement streaming replay decoder that never allocates from untrusted lengths before bounds checks.
- Integrate append/replay with M09-PR03 writer and logical in-memory apply model.

### Non-goals

- No snapshot format.
- No compression unless benchmarks justify a bounded optional frame in this PR.
- No encryption.
- No 1.x enum discriminant preservation or decoder.

### Acceptance evidence

- Random logical transactions round-trip byte-identically under canonical encoding and rebuild equivalent state.
- Arbitrary bytes/length/count/nesting/checksum/unknown-version inputs never panic/OOM/hang and return typed errors.
- Torn final record is recoverable according to protocol; interior/sealed corruption is not silently truncated.
- Reordered/missing generation/reference transactions fail before state mutation.
- WAL replay integrates with mixed edges, catalog, constraints/index metadata, and stable IDs.
- No physical row or Rust layout appears in the format.

### Tests and gates

- Round-trip/property tests across all logical change variants.
- Golden binary fixtures with documented version/hash.
- Corruption/torn-tail/unknown-version/limit fixtures.
- Fuzz decoder with bounded max length and corpus.
- Replay generation/reference validation tests.
- Mutation tests for checksums/bounds/classification.

### Review focus

- Safe bounded decode.
- Semantic not physical encoding.
- Torn-tail versus corruption classification.
- Version/unknown field policy.

### Stop conditions

- Codec choice requires unsafe code or unbounded allocations.
- Unknown field/version policy can skip authoritative semantics.
- Logical model lacks a required transaction invariant.

### Bridge and deletion

- No 1.x codec bridge.
- Golden 2.0 fixtures become permanent compatibility tests within 2.x only.

<a id="m09-pr05"></a>
## M09-PR05 — Implement Snapshot Format 2.0 and Coherent Catalog/Graph Checkpoints

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR04, M08-PR05
- **Issues:** None
- **Commit scope:** `snapshot`

Write and load a sectioned snapshot containing catalog descriptors and all named graph primary data at one transaction cut, with independent checksums, bounded decoding, and rebuild metadata.

### Scope

- Define snapshot header and section directory with StoreId/epoch, format/profile/collation identity, checkpoint transaction sequence, catalog generation, graph generations, counts/limits, and per-section checksums.
- Define authoritative sections for catalog root/schemas/object descriptors, graph metadata, nodes, edges/directionality/endpoints, labels, properties, graph types, constraints/index registrations, and ID allocator state.
- Define optional/rebuildable sections only where safe; initial vector/text/JSON/algorithm accelerator internals remain omitted/rebuilt.
- Implement coherent snapshot capture under the single-writer/immutable-reader model without combining different generations.
- Implement bounded streaming load/validation and construct unpublished database state before atomic open publication.
- Add golden fixtures, decoder fuzz, and size/throughput benchmarks.

### Non-goals

- No incremental snapshot or page-level copy-on-write.
- No online snapshot across concurrent writer without the existing serialized publication point.
- No 1.x snapshot decoder.
- No backup upload/service.

### Acceptance evidence

- Multi-schema/multi-graph mixed-edge database snapshot/reopen is semantically equivalent at the checkpoint sequence.
- Dense rows may differ after load while stable IDs/references/query results remain correct.
- Corrupt/unknown/oversized/inconsistent sections fail without partial database publication.
- Derived index state rebuilds from registrations/primary data and does not become a second authority.
- Snapshot and WAL replay join at exactly one sequence/generation cut with no gap/overlap.
- Arbitrary decoder inputs never panic/OOM/hang.

### Tests and gates

- Full state round-trip/property tests.
- Row remap/ID stability tests.
- Section corruption/unknown/limit/referential fixtures.
- Snapshot+WAL boundary tests.
- Failure injection across temp write/sync/rename/manifest steps.
- Snapshot decoder fuzz and mutation tests.

### Review focus

- One coherent cut.
- Stable IDs versus rows.
- Safe section/version handling.
- Derived state not authoritative.

### Stop conditions

- Snapshot capture combines catalog and graph generations from different cuts.
- Loader publishes state before complete validation.
- A derived provider cannot rebuild from authoritative data/metadata.

### Bridge and deletion

- No 1.x snapshot bridge.
- Current snapshot structs remain archive-only until M09-PR08 deletion.

<a id="m09-pr06"></a>
## M09-PR06 — Implement Checkpoint, WAL Rotation, Retention, and Manifest Publication Protocol

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR05
- **Issues:** None
- **Commit scope:** `persist`

Coordinate snapshot creation, WAL segment sealing/rotation, manifest lineage, CURRENT publication, retention, and deletion with crash-safe ordering and one anchored authority.

### Scope

- Implement checkpoint fence selection, snapshot temp write/sync/rename, new WAL segment creation/seal, manifest generation write/sync/rename, CURRENT update, and post-publication pruning.
- Define replay lineage: selected snapshot plus exact ordered WAL segment/sequence range.
- Implement retention policy over manifest/snapshot/WAL/archive generations while preserving every artifact reachable from current and configured fallback manifests.
- Make deletion/prune best-effort after authoritative publication and surface warnings without invalidating a successful checkpoint unless policy requires.
- Add crash hooks at every artifact operation and deterministic state enumerator/classifier.
- Integrate manual/threshold checkpoint entry points through Database/StoreDirectory.

### Non-goals

- No remote backup.
- No point-in-time recovery UI beyond retained generations.
- No compaction of WAL records in place.
- No 1.x artifact cleanup.

### Acceptance evidence

- Crash at every step leaves at least one valid recoverable lineage or a typed corruption error when the initial store never completed.
- Recovery selected from CURRENT never observes missing segment gaps/overlaps or a snapshot beyond WAL cut.
- Retention never deletes artifacts reachable from current/fallback manifests and eventually removes unreachable artifacts after success.
- Rename/replacement capability tests remain green across checkpoint/prune.
- Warnings versus fatal checkpoint outcomes are documented and status-mapped.
- Repeated checkpoints/rotations/reopens are deterministic and bounded.

### Tests and gates

- Exhaustive checkpoint crash/failure matrix.
- Manifest reachability/retention property tests.
- Segment gap/overlap/reorder fixtures.
- Repeated threshold/manual checkpoint integration.
- Directory capability and lock tests.
- Mutation tests around publication/prune ordering.

### Review focus

- Publication/prune ordering.
- Reachability-based retention.
- Lineage gap/overlap checks.
- One authority/capability.

### Stop conditions

- Any crash point can leave CURRENT naming an incomplete lineage.
- Pruning depends only on timestamps/names.
- Checkpoint blocks writer beyond accepted budget without evidence/plan.

### Bridge and deletion

- No legacy checkpoint path after this PR for format 2.
- Format 1 cleanup is never attempted.

<a id="m09-pr07"></a>
## M09-PR07 — Implement Recovery State Machine, Corruption Classification, and Crash Matrix

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR06
- **Issues:** None
- **Commit scope:** `recovery`

Build one deterministic open/recovery pipeline that validates store control files, loads the selected snapshot, replays WAL, rebuilds derived state, verifies invariants, and publishes the database only on complete success.

### Scope

- Implement recovery phases: anchor/lock, inspect header/version, read CURRENT, validate manifest lineage, load snapshot, replay WAL, rebuild derived providers, verify catalog/graph/index/constraint invariants, publish DatabaseInner.
- Define typed state/phase/error/report objects with artifact/offset/sequence/status context and safe diagnostic messages.
- Classify missing optional, torn unsealed tail, checksum corruption, structural corruption, unsupported version/profile, gap/overlap, foreign StoreId, stale generation, rebuild failure, and invariant failure.
- Guarantee recovery is non-destructive by default; no automatic truncation/repair of ambiguous corruption.
- Run generated crash states from checkpoint/commit hooks and assert recovered transaction sets/outcomes.
- Expose an administrative verify/report mode through facade without opening for writes.

### Non-goals

- No salvage/repair command.
- No 1.x import.
- No remote recovery.
- No hidden best-effort continuation after authoritative corruption.

### Acceptance evidence

- Every generated crash point has an expected recovered transaction set and no “maybe” classification unless the commit phase was genuinely indeterminate.
- Corruption classes are deterministic and non-destructive; files remain inspectable after failure.
- Database is never externally reachable before complete recovery/verification.
- Active constraints cannot open with incomplete backing indexes.
- Verify/report mode returns structured evidence without mutating artifacts.
- Recovery latency and memory are recorded for snapshot-heavy and WAL-heavy cases.

### Tests and gates

- Generated crash matrix across commit/checkpoint/publication phases.
- Corruption/torn/gap/version/store-ID/rebuild fixtures.
- No-partial-publication concurrency tests.
- Provider rebuild/constraint activation tests.
- Recovery decoder fuzz and mutation tests.
- Verify-report snapshot tests.

### Review focus

- Torn-tail boundary and non-destructive corruption policy.
- No partial publication.
- Constraint/provider rebuild authority.
- Crash matrix completeness.

### Stop conditions

- Any corruption case is “fixed” destructively without explicit repair design.
- Recovery exposes partial Database state.
- Commit/checkpoint crash hooks do not cover a publication phase.
- Provider degraded-open policy is ambiguous.

### Bridge and deletion

- No legacy recovery path for normal open.
- M09-PR08 removes all 1.x codecs and finalizes unsupported detection.

<a id="m09-pr08"></a>
## M09-PR08 — Cut Over Exclusively to Format 2.0 and Remove Legacy Persistence Bridges

- **Owner:** M09
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M09-PR07
- **Issues:** None
- **Commit scope:** `persist`

Make format 2 the only writable/readable runtime format, detect 1.x only enough to return a typed unsupported error, and delete all legacy WAL/snapshot/manifest/recovery/adapters.

### Scope

- Inventory and delete all production 1.x writer/reader/manifest/snapshot/audit/recovery structs, codecs, magic constants, compatibility adapters, and feature flags, including the legacy encoded bare-ID `selene_core::Value` graph/node/edge reference variants and codecs after M05-PR03 carrier migration.
- Implement a minimal header/version probe that recognizes known 1.x artifacts/store roots and returns `UnsupportedFormat` with found/expected version and rebuild guidance without decoding data.
- Route all database create/open/checkpoint/commit/recovery through the format 2 authority.
- Update public errors, docs, examples, tests, release artifacts, and package features to remove migration/compatibility expectations.
- Retain selected 1.x byte fixtures only in a test data directory to prove rejection/non-mutation.
- Run full crash/fuzz/format/public API gates and establish format 2 compatibility baseline within 2.x.

### Non-goals

- No export/import tool.
- No automatic or manual in-place migration.
- No writing format 1.
- No promise that every historical corrupt 1.x file is recognized.

### Acceptance evidence

- Repository production code contains no 1.x decoder/writer/recovery branch, compatibility feature, or legacy encoded bare-ID `Value` reference variant/codec.
- Known 1.x fixtures return the typed unsupported error and their directory hashes are unchanged after open attempts.
- All format 2 create/commit/checkpoint/reopen/crash/fuzz tests pass through the sole path.
- Public docs/errors consistently state no migration and no 1.x support.
- Format 2 golden fixtures and compatibility policy are recorded for future 2.x releases.
- M09 milestone exit and full release persistence gates pass.

### Tests and gates

- Known 1.x rejection/non-mutation fixtures.
- Repository stale legacy magic/type/feature search.
- Full format 2 round-trip/crash/recovery suite.
- All persistence fuzz targets and mutation tests.
- Public API/package feature snapshots.
- Cross-platform release workflow persistence jobs.

### Review focus

- Legacy code is actually gone.
- Unsupported open is bounded and non-mutating.
- No accidental migration promise.
- 2.x format compatibility policy.

### Stop conditions

- A production component still depends on a legacy payload/codec.
- Unsupported detection mutates/locks the old directory in a way that violates policy.
- Format 2 crash/fuzz evidence is not green.

### Bridge and deletion

- Delete every legacy persistence bridge, including encoded bare-ID `selene_core::Value` graph/node/edge reference variants and codecs after M05-PR03 removes runtime dependence.
- Test-only 1.x fixtures remain solely to assert safe rejection.

<a id="m10-pr01"></a>
## M10-PR01 — Reintegrate Graph Algorithms through Catalog Graph Handles and Batch Results

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M06-PR07, M07-PR06, M09-PR08
- **Issues:** None
- **Commit scope:** `algorithms`

Move algorithms and projections onto database-owned graph handles, mixed-edge policies, typed candidates, batch inputs/results, and facade procedure calls without exposing storage rows.

### Scope

- Replace direct `SharedGraph`/row projection entry points with internal `GraphReadHandle`, snapshot generation, typed candidate sets, and explicit direction policy.
- Migrate CSR/projection construction to stable IDs/private dense mapping and validate mixed/directed/undirected/symmetrized/reject modes per algorithm.
- Return typed binding batches/values and structured diagnostics through `algo.*` procedure descriptors.
- Register projection/catalog dependencies and rebuild/invalidation behavior under graph generation changes and recovery.
- Preserve exact/reference implementations and existing algorithm correctness tests while updating API ownership.
- Expose only selected facade convenience methods; lower-level algorithm crate remains advanced/unstable unless deliberately re-exported.

### Non-goals

- No new algorithm families.
- No distributed/GPU algorithms.
- No persistent projection as authoritative state.
- No generic plugin ABI.

### Acceptance evidence

- All selected existing algorithms execute through facade/procedure paths on named graphs after reopen.
- No public algorithm API exposes graph rows/bitmaps/concrete storage.
- Mixed-edge policy tests exist for every algorithm/projection entry point.
- Projection invalidates/rebuilds on graph generation changes and recovery.
- Results match existing reference/oracle tests and preserve deterministic behavior where promised.
- Legacy graph-root algorithm adapters are deleted.

### Tests and gates

- Algorithm correctness/reference corpus.
- Mixed-edge policy and stable-ID projection tests.
- Generation/recovery invalidation tests.
- Batch result descriptor tests.
- Cancellation/resource tests.
- Public API/legacy adapter search.

### Review focus

- Projection-local identity.
- Mixed-edge mathematical policy.
- No storage leakage.
- Recovery/generation behavior.

### Stop conditions

- An algorithm has no defensible mixed-edge policy; reject unsupported input and document rather than guess.
- Projection cache becomes commit-critical/authoritative.
- Legacy adapter remains required by public facade.

### Bridge and deletion

- Delete legacy `GraphAlgorithms` direct graph-root convenience if it leaks 1.x ownership.
- Test-only direct/reference helpers may remain internal.

<a id="m10-pr02"></a>
## M10-PR02 — Reintegrate Vector Values, Indexes, and Search through the 2.0 Catalog

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M08-PR01, M09-PR08, M06-PR06
- **Issues:** None
- **Commit scope:** `vector`

Move vector exact/ANN capabilities onto catalog-owned index descriptors, transaction maintenance, format 2 registrations, typed batch procedures, and exact reranking under the facade.

### Scope

- Migrate vector index registrations (exact/HNSW/IVF/TurboQuant as retained) to stable catalog IndexId/target/metric/dimension/build-state descriptors.
- Route create/drop/rebuild/stats/search/score procedures through catalog, graph handles, typed candidates, batches, and transaction/recovery provider interfaces.
- Rebuild ANN state from primary vector properties after snapshot/WAL recovery and validate descriptor dimension/metric/function/profile identity.
- Guarantee approximate candidate paths exact-rerank against primary vectors before returning final distances where current contract promises it.
- Integrate expression/property target and graph/state/edge filter candidates without raw rows.
- Remove old global registration maps/direct graph APIs and establish 2.0 vector correctness/performance baselines.

### Non-goals

- No new ANN algorithm unless required to keep existing retained features correct.
- No GPU/WGPU production commitment.
- No external embedding service in CI.
- No authoritative persistence of rebuildable ANN internals.

### Acceptance evidence

- Retained vector DDL/procedures work through `selene-db` on named graphs before/after reopen.
- ANN result distances/top-k match exact rerank contract and recall benchmarks remain documented.
- Index build/rebuild/drop/recovery state transitions are catalog-consistent and generation-safe.
- Dimension/metric/type/filter errors return structured diagnostics.
- No raw rows/direct SharedGraph/global registry remain in public/procedure contracts.
- Exact and approximate benchmark rows include index storage plus primary vector storage and source SHA/date.

### Tests and gates

- Exact/ANN differential and recall tests.
- Catalog lifecycle/provider rebuild/recovery tests.
- Metric/dimension/type/filter/candidate tests.
- Transaction maintenance create/update/delete tests.
- Batch procedure result tests.
- Mutation tests around rerank/provider selection.

### Review focus

- Exact rerank and source of truth.
- Catalog/provider state machine.
- No row leakage.
- Truthful benchmark/recall reporting.

### Stop conditions

- An ANN provider cannot rebuild deterministically from authoritative data/descriptor.
- Approximate path returns final scores without exact contract where required.
- CI would require secrets/live service.

### Bridge and deletion

- Delete old vector registration/global graph adapters.
- Exact oracle remains permanent.

<a id="m10-pr03"></a>
## M10-PR03 — Reintegrate BM25 Text, JSON Search, and Maintained Native Providers

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M08-PR05, M09-PR08, M06-PR06
- **Issues:** None
- **Commit scope:** `retrieval`

Move text and JSON native capabilities to catalog-owned descriptors, typed candidates, batch procedures, transaction maintenance, and recovery rebuild while preserving exact oracles.

### Scope

- Migrate text index registrations/analyzer parameters and JSON expression index registrations to catalog IndexIds/provider lifecycle.
- Route exact BM25/JSON scans, maintained text lookup, candidate scoring, batch/state-expanded scoring, JSON path lookups, rebuild/stats/drop through graph handles and batch procedures.
- Maintain/rebuild text and JSON-derived indexes on transaction/recovery/compaction with primary string/JSON values authoritative.
- Unify typed candidate filters from graph reachability, scalar/expression indexes, edge filters, and maintained candidate state.
- Preserve exact scan/evaluator as correctness oracle and explicit fallback with plan/procedure observability.
- Delete direct graph/global registration/row interfaces and refresh GQL extension/evidence docs.

### Non-goals

- No new full-text analyzer ecosystem or disk segment engine.
- No GIN JSON containment index.
- No grammar shortcuts.
- No external search service.

### Acceptance evidence

- All retained text/JSON procedures work through facade/catalog on named graphs before/after reopen.
- Maintained provider results match exact oracle for randomized corpora and updates.
- Analyzer/path/type/config changes invalidate/rebuild safely.
- Candidate composition across graph/scalar/edge/state filters has exact set semantics.
- No raw row/global graph registration surfaces remain.
- Benchmarks demonstrate scan-to-index graduation and include rebuild/memory costs.

### Tests and gates

- Exact versus maintained differential/property tests.
- Transaction maintenance/recovery/rebuild tests.
- Analyzer/path/config invalidation tests.
- Candidate composition and mixed-edge direction tests.
- Batch procedure schema/status tests.
- Mutation tests for provider/fallback selection.

### Review focus

- Exact oracle parity.
- Descriptor version/invalidation.
- Candidate set semantics.
- No scope creep into disk/search service.

### Stop conditions

- Maintained provider cannot prove exact parity on selected semantics.
- Analyzer configuration is not persistently identifiable.
- A provider is treated as authoritative.

### Bridge and deletion

- Delete old text/JSON registration/direct graph adapters.
- Exact scan/evaluator remains permanent oracle/fallback.

<a id="m10-pr04"></a>
## M10-PR04 — Complete Named Procedure Catalog and Selene Extension Profile

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M10-PR01, M10-PR02, M10-PR03, M02-PR03
- **Issues:** None
- **Commit scope:** `procedure`

Promote native procedures to catalog objects with stable descriptors, generation-aware resolution, capability-restricted execution, and generated extension documentation without adding loadable extensions.

### Scope

- Define named procedure catalog descriptors with stable ID/name/schema, mandatory/optional parameter descriptors/defaults, result type, side-effect class, provider kind/version, and generation.
- Register built-in `selene.*` and `algo.*` procedures into system schema/catalog population through the same descriptor service.
- Resolve named calls through catalog IDs/generations and execute through the M06 procedure capabilities.
- Generate extension inventory, syntax/call examples, result schemas, side effects, and implementation-defined provider mechanism disclosures.
- Retain internal test registry injection but remove production global `BuiltinProcedureRegistry` construction requirements from embedders.
- Define system procedure upgrade/reconciliation on database open/profile version.

### Non-goals

- No user-created GQL procedure DDL unless selected separately.
- No dynamic library/WASM/plugin loader.
- No network RPC procedure.
- No third-party stable ABI.

### Acceptance evidence

- All retained native procedures resolve as catalog objects with generated descriptors and execute through batch/context APIs.
- Plan cache invalidates on procedure descriptor/provider generation change.
- Descriptor/implementation side-effect or result mismatch fails tests/open registration.
- System procedures cannot be accidentally dropped/overwritten through ordinary user DDL.
- Embedders no longer construct/pass a production BuiltinProcedureRegistry for normal execute calls.
- Generated extension docs/manifest exactly match registered procedure descriptors.

### Tests and gates

- System catalog population/reconciliation tests.
- Descriptor/implementation signature/effect/result validation.
- Resolution/cache invalidation tests.
- Protected object DDL tests.
- Generated extension manifest/docs golden tests.
- Test registry isolation tests.

### Review focus

- Catalog primary object semantics.
- Capability/descriptor enforcement.
- No plugin ABI creep.
- Generated extension truth.

### Stop conditions

- A current procedure signature cannot be represented without exposing internal types.
- System catalog population conflicts with transaction/recovery model.
- A test registry path can affect production claim state.

### Bridge and deletion

- Delete production BuiltinProcedureRegistry parameter/adapters.
- Internal test registry remains non-public and claim-isolated.

<a id="m10-pr05"></a>
## M10-PR05 — Close the Selected GQL Profile and Generate the Release Conformance Declaration

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M10-PR04, M01-PR06
- **Issues:** None
- **Commit scope:** `conformance`

Complete rule evidence, feature implication closure, implementation-defined/extension disclosures, and claim wording for the exact 2.0 release profile—or explicitly ship as ISO-aligned with a generated blocker list if any mandatory evidence remains.

### Scope

- Re-run and review the complete target profile, transitive implication closure, Annex B applicability/values, Unicode/collation identity, extension inventory, and rule evidence.
- Complete missing positive/negative/status/type/ordering/side-effect/model/persistence tests for every mandatory and claimed rule.
- Resolve every `claimed_pending_evidence`, stale symbol, placeholder, duplicate, unknown dependency, and overclaim.
- Generate signed/hash-bound machine and Markdown conformance declarations with exact feature lists, supported property types, implementation-defined choices, extensions, limitations, and test command/results.
- Set release wording from gate result only: formal selected-profile/minimum claim when complete, otherwise “ISO/IEC 39075:2024-aligned” plus exact blockers.
- Add the complete claim gate to release CI and prevent manual stronger wording in README/release notes.

### Non-goals

- No external certification.
- No optional-feature implementation solely to make a larger claim.
- No hiding failed/pending evidence.
- No reproduction of normative text.

### Acceptance evidence

- Transitive implication closure is green for every claimed feature.
- Every mandatory/claimed rule has complete executable evidence and expected status/type/side-effect assertions, or claim wording remains explicitly incomplete.
- Annex B applicable records have final values/evidence and no IA001 or other mapping mismatch.
- Generated declaration includes SHA/profile hash/test result hash/tool version and is reproducible on a clean checkout.
- README/release notes cannot claim more than the generated declaration.
- The independent reviewer pair verifies at least a sample of each major clause/evidence category before PASS.

### Tests and gates

- Complete conformance suite and claim gate.
- Generated artifact reproducibility/clean-tree tests.
- Overclaim negative fixtures.
- Stale symbol/evidence reference checks.
- Feature flagger/profile status corpus.
- Release workflow non-publishing dry run.

### Review focus

- Evidence completeness versus labels.
- Implication closure and Annex B accuracy.
- Scoped wording.
- Reproducible hash-bound artifact.

### Stop conditions

- Any mandatory/claimed rule remains pending but docs attempt a formal claim.
- A feature implication or Annex B mapping cannot be verified.
- Release test results are not reproducible from the candidate SHA.

### Bridge and deletion

- No bridge. This PR freezes the release profile for the candidate.
- Post-GA feature additions require new profile/evidence transitions.

<a id="m10-pr06"></a>
## M10-PR06 — Finalize Public API, Documentation, Examples, and No-Migration Release Guidance

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** High / L
- **Dependencies:** M10-PR05
- **Issues:** None
- **Commit scope:** `docs`

Make the 2.0 facade, errors, rustdoc, examples, package metadata, architecture docs, and upgrade guidance coherent and remove every remaining prototype/1.x/bridge reference from the supported surface.

### Scope

- Review and freeze the intended stable `selene-db` facade surface; internalize/remove accidental lower-crate exports and temporary alpha names.
- Complete rustdoc for Database/Builder/Session/Request/Params/Outcome/catalog/transaction/graph handles/candidates/index/constraint/procedure APIs.
- Rewrite getting started, embedding, catalog, transaction, query, path, persistence, retrieval, algorithms, conformance, errors, operations, and performance docs.
- Provide end-to-end examples for in-memory and persistent databases, multi-schema/graph, explicit transaction, constraints/indexes, JSON/vector/text, paths, algorithms, recovery/reporting.
- Publish explicit 1.x EOL/no binary or source compatibility/no persisted migration guidance and rebuild/import approach.
- Validate package names/features/dependencies/licenses/readmes/keywords and crates.io dry-run order.

### Non-goals

- No compatibility crate.
- No 1.x migration utility.
- No new feature implementation except doc-discovered correctness fixes split into focused PRs.
- No server tutorial.

### Acceptance evidence

- Public API review/snapshot contains only intended stable types and no temporary bridge/internal concrete type.
- All examples/doctests compile and run in CI where appropriate.
- Repository search finds no active 1.x/prototype/row executor/legacy format/old session/registry guidance.
- Crates.io package dry runs and dependency publish order succeed without network publication.
- No-migration/EOL wording is consistent across README/CHANGELOG/docs/errors.
- Generated conformance and benchmark content is current at candidate SHA.

### Tests and gates

- Public API snapshot and cargo-semver/public-api review (informational within alpha).
- All examples/doctests/link checks.
- Package/crates.io dry-run scripts.
- Repository stale-term/bridge searches.
- Docs code-block tests.
- License/notice/third-party checks.

### Review focus

- Facade stability and internal leakage.
- Executable examples.
- No-migration clarity.
- No stale/overstated conformance/performance content.

### Stop conditions

- Documentation reveals an API correctness defect; split a focused code PR and block this PR.
- A lower-crate type is required publicly without a stability decision.
- Crates.io packaging differs from documented dependency graph.

### Bridge and deletion

- Delete all temporary alpha aliases/bridge documentation.
- Archive historical review notes but remove them from active user navigation.

<a id="m10-pr07"></a>
## M10-PR07 — Run Release Candidate Hardening, Final Review, Tag, and Publish

- **Owner:** M10
- **State:** Unmerged
- **Risk / size:** Critical / L
- **Dependencies:** M10-PR06
- **Issues:** None
- **Commit scope:** `release`

Produce a clean release candidate SHA, execute the complete cross-platform correctness/durability/conformance/performance/security gate, obtain independent exact-head PASS, and only then perform separately authorized merge, tag, and publication actions.

### Scope

- Freeze the candidate profile/API/format and allow only release-blocking fixes as separate reviewed commits/PRs.
- Run Linux/macOS full build, clippy, nextest, doctest, rustdoc, deny, audit, license/notice, secret/file-size/row/version/doc/bench checks.
- Run complete parser/persistence/plan fuzz, selected mutation suites, generated crash matrix, conformance claim gate, public API/package dry runs, and benchmark guard set.
- Generate release evidence bundle with SHA/toolchains/commands/results/hashes/profile/conformance/format/API/benchmark/crash summaries.
- Return the final development→main release handoff to the orchestrator, which owns the non-draft PR and any eligible authorized merge.
- After exact-head PASS and authorized merge, the repository owner performs the separately authorized semver tag/publication actions and verifies dependency order plus facade.

### Non-goals

- No new product feature.
- No benchmark tuning without a regression/root-cause PR.
- No bypassing failed fuzz/mutation/crash/conformance gates.
- No self-approval, auto-merge, or implicit tag/publication authority.

### Acceptance evidence

- Every required gate is green or a formally accepted non-blocking exception is recorded in release evidence and does not undermine correctness/conformance claim.
- Crash matrix, fuzz, mutation, and conformance artifacts are tied to the exact candidate SHA.
- Benchmark guards show no unexplained regression versus accepted 2.0 section baselines.
- The independent review pair returns PASS on the unchanged exact head with no unresolved Blocker/Major findings.
- The orchestrator merges only after every eligibility condition and explicit user authorization; separately authorized tag/publication then succeeds in dry-run and actual workflows.
- Published packages, docs, and generated conformance declaration report the same version/profile/SHA lineage.

### Tests and gates

- All repository/full release/nightly gates.
- Complete conformance and crash matrices.
- Fuzz budgets: release short plus nightly/deep soak as defined.
- Selected mutation suites with thresholds.
- Crates.io/package smoke after publication.
- Install/use smoke in a fresh minimal consumer project.

### Review focus

- No scope creep or bypass.
- Evidence tied to exact SHA.
- Conformance wording and format/API freeze.
- Orchestrator merge eligibility and separately controlled tag/publication.

### Stop conditions

- Any unexplained correctness, crash, fuzz, mutation, conformance, security, or material performance failure.
- Candidate changes after evidence without rerun.
- The final independent verdict is FIX or REPLAN.
- Publish workflow/tag classification is not proven.

### Bridge and deletion

- No bridges may remain in stable production code.
- Post-GA work begins from new milestones/PRs; 1.x remains EOL.
