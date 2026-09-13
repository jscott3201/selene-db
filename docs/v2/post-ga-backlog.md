# Post-GA and deferred backlog

The original items below are outside the 65-work-item initial program. The
F06-PR01 section records additional owner-approved alpha deferrals. Their absence
is a scope decision, not work to absorb opportunistically or a denial of defects
in agreed behavior.

| Deferred item | Boundary |
|---|---|
| Network server and wire protocol | 2.0 remains embedded; protocol, auth, and connection semantics need separate architecture. |
| Bundled user administration | The engine exposes embedder policy, not an auth service. |
| MVCC, multiple writers, weaker isolation | Keep serializable single-writer publication until workload evidence requires redesign. |
| Nested directories or multiple catalogs | Initial catalog is one synthetic root with schemas only. |
| 1.x reader, import, or migration | Rebuild from source data; no compatibility path is planned. |
| Loadable dynamic or WASM ABI | Native procedures stay in-tree; a third-party ABI needs security and versioning design. |
| Distributed query or graph execution | No clustering, sharding, replication, or distributed transactions. |
| JIT or data-centric code generation | Build and measure the batch interpreter first. |
| Join/group/sort disk spill | Initial engine uses bounded memory and typed resource errors. |
| JSON containment index | Implement deterministic scalar-path expression indexes first. |
| Weighted path language extensions | Keep standard path semantics separate from namespaced algorithms. |
| Durable ANN/text accelerator internals | Derived providers rebuild from primary data and registrations. |
| Production GPU execution | Experimental benchmark rows are not release requirements. |
| Every optional GQL feature | Admit only implication-closed, evidence-complete profile increments. |

<a id="entry-criteria"></a>

## Entry criteria

A deferred item enters planning only after focused research or an ADR defines
need, ownership, public and persisted compatibility, conformance impact,
safety/threat model, evidence, and PR-sized slices.

## F06-PR01 alpha scope decision

Justin decided on 2026-09-13 to scope 2.0-alpha to the completed finish-plan subset
and defer the remaining canonical-target families, rule-inventory completion and
pending Annex B decisions below to this post-GA backlog. This is a prose-only
release-scope decision, not a change to the canonical target or ISO minimum.
Already executing subsets and already decided, evidenced choices remain intact.
See [release readiness](release-readiness.md#known-gaps-no-silent-scope-decision)
for the acceptance boundary and denied-by-design formal `selected_profile` claim.

<a id="deferred-canonical-target-families"></a>

### Deferred canonical-target families

Boundaries follow each feature's `unsupported_rationale` in
`spec/gql-profile/profile.json`; GV65 has an empty rationale and is explicitly
`referenced` with claim state `unsupported`. Syntax observation or facade reference
carriers do not establish the corresponding ISO runtime semantics. These rows do
not remove any feature or implication from the canonical target.

| Deferred item | Boundary |
|---|---|
| GC03 — Conditional graph-type lifecycle | Today: conditional lifecycle executes for property-free named-node types. Deferred: properties, edges, explicit key labels, and `COPY OF` / `LIKE` / external sources. Entry: [entry criteria](#entry-criteria). |
| GE04 — Graph parameters | Today: parameter syntax is observed, not complete graph-parameter execution. Deferred: graph parameters backed by GV60 graph reference values. Entry: [entry criteria](#entry-criteria). |
| GV60 — Graph reference value types | Today: `GRAPH` reference type spellings parse to the Flagger. Deferred: graph reference value semantics and closed graph constraints. Entry: [entry criteria](#entry-criteria). |
| GE05 — Binding table parameters | Today: parameter syntax is observed, not complete binding-table parameter execution. Deferred: binding table parameters backed by GV61 reference values. Entry: [entry criteria](#entry-criteria). |
| GV61 — Binding table reference value types | Today: no runtime support is claimed by the profile. Deferred: `TABLE` field-type descriptors and binding-table value semantics required for reference types. Entry: [entry criteria](#entry-criteria). |
| GG02 — Closed graph types | Today: named closed graphs execute for property-free node-only graph types. Deferred: properties, edges and endpoints, explicit key labels, and other graph-type sources; richer Rust schema construction does not complete this GQL grammar. Entry: [entry criteria](#entry-criteria). |
| GG20 — Explicit element type names | Today: named property-free node types execute in the bounded catalog subset. Deferred: named edge types and complete element declarations. Entry: [entry criteria](#entry-criteria). |
| GG21 — Explicit element type key label sets | Today: the executable node-only subset derives one key label from each node type name; explicit key-label-set forms reject. Deferred: explicit element key-label-set forms. Entry: [entry criteria](#entry-criteria). |
| GP16 — `AT` schema clause | Today: bounded absolute schema references at read-procedure heads execute. Deferred: other schema-reference forms, composed nested bodies and non-query procedure bodies. Entry: [entry criteria](#entry-criteria). |
| GQ01 — `USE` graph clause | Today: bounded catalog-backed focused read queries and single-linear-body nesting execute. Deferred: focused mutations, multiple focused parts and graph-valued bindings; GT03 remains unsupported. Entry: [entry criteria](#entry-criteria). |
| GV65 — Dynamic union types | Today: a referenced implication target with unsupported claim state, not an implemented ISO dynamic-union surface. Deferred: the underlying dynamic union type surface required by GV66 and GV67. Entry: [entry criteria](#entry-criteria). |
| GV66 — Open dynamic union types | Today: open dynamic union syntax is observed. Deferred: runtime semantics over the unimplemented GV65 surface; syntax admission is not execution evidence. Entry: [entry criteria](#entry-criteria). |
| GV67 — Closed dynamic union types | Today: closed dynamic union syntax is observed. Deferred: runtime semantics over the unimplemented GV65 surface; normalized structural descriptors do not establish this language feature. Entry: [entry criteria](#entry-criteria). |

<a id="deferred-rule-inventory-completion"></a>

### Deferred rule-inventory completion

| Deferred item | Boundary |
|---|---|
| Complete the `seeded_incomplete` rule inventory | Today: `spec/gql-profile/rules.json` and `spec/gql-profile/evidence.json` register seed contracts and a pending inventory marker, not complete positive/negative semantic coverage. Deferred: complete normative rule/applicability inventory, alternative-choice and implication evidence, and executed semantic evidence for the canonical target, with current ownership and source references. Existing regression tests and static `complete` dispositions are not a complete claim or hand-authored pass records. Entry: [entry criteria](#entry-criteria), including evidence dimensions and an executed-results plan before any claim transition. |

<a id="deferred-annex-b-decisions"></a>

### Deferred Annex B decisions

Each row records a currently `pending` decision in `spec/gql-profile/profile.json`,
not a finding that every related operation is absent or incorrect. The pending
reasons and historical owners need current-source review when admitted; this note
does not select values or rewrite those records. Already selected and evidenced
choices are not deferred: for example, catalog NFC (IW023), concatenation (IW017),
collation (ID022/IA015) and common-supertype selection (IW019) remain as recorded,
without closing profile-wide normalization or numeric/type-precedence gaps.

| Deferred item | Boundary |
|---|---|
| IA003 — Operations on unnormalized strings | Per-operation normalization exists; the reviewed profile-wide rule remains deferred. Entry: [entry criteria](#entry-criteria). |
| IA005 — Assignment digit loss | Defer the unified assignment-conversion and precision-loss rule, not already evidenced numeric representations. Entry: [entry criteria](#entry-criteria). |
| IA006 — Numeric approximations | Defer one reviewed coercion rule across literals, operators, casts and assignment. Entry: [entry criteria](#entry-criteria). |
| IA007 — Non-exact values with approximations | Defer the decision identifying which non-exact value families admit approximations. Entry: [entry criteria](#entry-criteria). |
| IA010 — Arithmetic operating bounds | Bounds exist across native integer, decimal and floating representations; defer their unified reviewed rule. Entry: [entry criteria](#entry-criteria). |
| IA011 — Approximate division digit loss | Defer the division-conversion policy within the unified numeric rules. Entry: [entry criteria](#entry-criteria). |
| IA017 — Repeated property assignments | Defer selecting and evidencing one repeated-assignment rule for batch mutations. Entry: [entry criteria](#entry-criteria). |
| IA019 — Bidirectional controls in literals | Defer an explicit source-syntax security rule for bidirectional controls, not the already selected identifier repertoire. Entry: [entry criteria](#entry-criteria). |
| IA021 — Numeric assignment overflow | Defer alignment and evidence of overflow handling across every numeric family. Entry: [entry criteria](#entry-criteria). |
| ID004 — Empty constructed-value element types | Defer the descriptor rule when no concrete element is present. Entry: [entry criteria](#entry-criteria). |
| ID005 — Elements-function declared type | Defer the reviewed derivation of result descriptors from the structural type model. Entry: [entry criteria](#entry-criteria). |
| ID062 — Non-negative integer specification type | Syntax fields use several host widths; defer one structural declared-type rule. Entry: [entry criteria](#entry-criteria). |
| ID063 — Mixed approximate arithmetic result type | Promotion exists; defer review and evidence for every approximate operand pairing. Entry: [entry criteria](#entry-criteria). |
| ID064 — Exact arithmetic result type | Defer one descriptor-level integer/decimal promotion rule. Entry: [entry criteria](#entry-criteria). |
| ID065 — Exact addition/subtraction precision | Defer the structural numeric precision-derivation rule. Entry: [entry criteria](#entry-criteria). |
| ID066 — Exact multiplication precision | Defer the structural numeric precision-derivation rule. Entry: [entry criteria](#entry-criteria). |
| ID067 — Exact division precision and scale | Defer one checked descriptor rule for division precision and scale. Entry: [entry criteria](#entry-criteria). |
| ID074 — Exact expression result precision | Defer the expression-wide structural numeric precision rule and evidence. Entry: [entry criteria](#entry-criteria). |
| ID075 — Approximate expression result precision | Defer the expression-wide structural numeric precision rule and evidence. Entry: [entry criteria](#entry-criteria). |
| ID095 — Exact `SUM` result type | Defer exact aggregate result descriptors under a unified numeric promotion contract. Entry: [entry criteria](#entry-criteria). |
| ID096 — Exact `AVG` result type | Defer exact aggregate result descriptors under a unified numeric promotion contract. Entry: [entry criteria](#entry-criteria). |
| ID097 — Approximate `SUM` / `AVG` result types | Defer approximate aggregate result descriptors under a unified numeric promotion contract. Entry: [entry criteria](#entry-criteria). |
| ID098 — Standard-deviation result types | Defer statistical aggregate descriptors under a unified numeric promotion contract. Entry: [entry criteria](#entry-criteria). |
| ID099 — Binary aggregate result types | Defer binary aggregate descriptors under a unified numeric promotion contract. Entry: [entry criteria](#entry-criteria). |
| IE001 — URI/URL resource mapping | The facade has no selected catalog mapping for these values; defer that decision without implying a network server. Entry: [entry criteria](#entry-criteria). |
| IE009 — Additional informational conditions | Defer the decision defining implementation-added informational conditions in unified diagnostics. Entry: [entry criteria](#entry-criteria). |
| IE010 — Successful informational subclasses | Defer selection and evidence of a closed set of successful informational subclasses. Entry: [entry criteria](#entry-criteria). |
| IL023 — Approximate exponent bounds | Defer a consistent statement and evidence of normal/subnormal exponent bounds, not the selected IEEE representations. Entry: [entry criteria](#entry-criteria). |
| IV012 — Open dynamic union components | The canonical target selects open unions while the analyzer marks the ISO feature unsupported; defer the component decision alongside GV65/GV66. Entry: [entry criteria](#entry-criteria). |
| IW018 — Lax-cast generation | Defer the structural cast-matrix rule and companion type-test evidence together. Entry: [entry criteria](#entry-criteria). |
| IW021 — Type-precedence permutation | Defer a complete precedence order for common-type selection; the selected IW019 mechanism does not close this decision. Entry: [entry criteria](#entry-criteria). |
