# SeleneDB Conformance Corpus

This directory is the tracked home for claimed-feature query coverage.

Layout:

- `positive/` - queries that exercise claimed features and declare expected results.
- `negative/` - queries that require unclaimed features and declare expected GQLSTATUS rejections.
- `fixtures/` - graph and procedure-pack fixtures used by corpus entries.

File names start with the relevant ISO feature ID, for example
`GP04-named-procedure-call.gql` or `GT03-multi-graph-tx-rejected.gql`.

Each `.gql` file declares its expected outcome in header comments:

```sql
-- corpus: positive
-- feature: GP04
-- expects: ResultRows { columns: ["v"], rows: [[42]] }
-- fixture: person_graph

CALL count.nodes() YIELD count AS v
RETURN v
```
