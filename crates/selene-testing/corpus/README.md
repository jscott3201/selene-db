# SeleneDB Conformance Corpus

This directory is the tracked home for runtime-support query coverage.

Layout:

- `positive/` - queries that exercise runtime-supported features and parse cleanly.
- `negative/` - queries that require runtime-unsupported features and declare parser rejections.
- `fixtures/` - graph and procedure-pack fixtures used by corpus entries.

File names start with the relevant ISO feature ID, for example
`GP04-named-procedure-call.gql` or `GT03-multi-graph-tx-rejected.gql`.

Each `.gql` file declares its expected outcome in header comments. Headers use
the ISO GQL line-comment prefix `//` so the whole file is valid GQL source (the
SQL `--` comment is not part of ISO/IEC 39075:2024 and is rejected by the
parser):

```gql
// corpus: positive
// feature: GP04
// expects: parse-ok

CALL selene.count_nodes() YIELD count AS v
RETURN v
```

M5a corpus files are parse-only. Executor-backed `ResultRows` expectations
land with the planner/executor briefs.
