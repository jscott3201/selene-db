# Open issue ownership

Each issue open at installation has one 2.0 implementation owner. Foundation
work may reference an issue but must not close it before the owning PR produces
direct acceptance evidence.

<a id="issue-1088"></a>
- [#1088](https://github.com/jscott3201/selene-db/issues/1088) → [M09-PR01](roadmap/work-items-05-10.md#m09-pr01): anchored persistence directory capability.
<a id="issue-1092"></a>
- [#1092](https://github.com/jscott3201/selene-db/issues/1092) → [M08-PR02](roadmap/work-items-05-10.md#m08-pr02): named composite unique and key constraints.
<a id="issue-1093"></a>
- [#1093](https://github.com/jscott3201/selene-db/issues/1093) → [M04-PR02](roadmap/work-items-00-04.md#m04-pr02): typed generation-bound candidate sets and removal of row bitmap APIs.
<a id="issue-1094"></a>
- [#1094](https://github.com/jscott3201/selene-db/issues/1094) → [M08-PR03](roadmap/work-items-05-10.md#m08-pr03): incremental backing-index uniqueness enforcement.
<a id="issue-1097"></a>
- [#1097](https://github.com/jscott3201/selene-db/issues/1097) → [M08-PR05](roadmap/work-items-05-10.md#m08-pr05): JSON scalar-path expression indexes and pushdown.
<a id="issue-1128"></a>
- [#1128](https://github.com/jscott3201/selene-db/issues/1128) → [M09-PR03](roadmap/work-items-05-10.md#m09-pr03): WAL watermarks and precise commit outcomes.
<a id="issue-1137"></a>
- [#1137](https://github.com/jscott3201/selene-db/issues/1137) → [M08-PR06](roadmap/work-items-05-10.md#m08-pr06): read-hot map evidence and balanced performance gates.

Use `Closes #NNNN` only when the owner PR's tests or benchmarks demonstrate the
resolution. Moving ownership requires REPLAN and coordinated changes to the
issue map, machine plan, projections, and both affected contracts.
