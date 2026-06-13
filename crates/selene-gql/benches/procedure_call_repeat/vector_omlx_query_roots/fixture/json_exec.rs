use std::sync::Arc;

use selene_core::Value;
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};

use super::{OmlxGqlQueryRootFixture, TOP_K, db_string};

const JSON_CURRENT_VECTOR_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) MATCH (root)-[:OmlxSupports]->(candidate:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, anchor.query AS vector_query, collect_list(candidate) AS candidates GROUP BY anchor.query_index, anchor.query ORDER BY query_index CALL selene.json_path_contains_candidate_nodes('OmlxEmbeddingDoc', 'metadata', json_array('retrieval'), json('{\"current\":true,\"support\":true}'), candidates, 4096) YIELD node_id WITH vector_query, query_index, collect_list(node_id) AS candidates GROUP BY vector_query, query_index ORDER BY query_index WITH collect_list(vector_query) AS queries, collect_list(candidates) AS candidate_sets CALL selene.vector_score_nodes_batch('embedding', queries, candidate_sets, 4, 'cosine') YIELD query_index, node_id, distance RETURN query_index, node_id, distance";
const JSON_CURRENT_TEXT_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) MATCH (root)-[:OmlxSupports]->(candidate:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, anchor.query_text AS query_text, collect_list(candidate) AS candidates GROUP BY anchor.query_index, anchor.query_text ORDER BY query_index CALL selene.json_path_contains_candidate_nodes('OmlxEmbeddingDoc', 'metadata', json_array('retrieval'), json('{\"current\":true,\"support\":true}'), candidates, 4096) YIELD node_id WITH query_text, query_index, collect_list(node_id) AS candidates GROUP BY query_text, query_index ORDER BY query_index WITH collect_list(query_text) AS queries, collect_list(candidates) AS candidate_sets CALL selene.text_score_nodes_batch('OmlxEmbeddingDoc', 'body', queries, candidate_sets, 4) YIELD query_index, node_id, score RETURN query_index, node_id, score";

impl OmlxGqlQueryRootFixture {
    pub(crate) fn warm_query_root_json_current_vector_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_json_current_vector_batch_query(registry, Some(cache));
    }

    pub(crate) fn warm_query_root_json_current_text_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_json_current_text_batch_query(registry, Some(cache));
    }

    pub(crate) fn execute_json_current_vector_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        self.execute_json_batch_source(
            registry,
            cache,
            JSON_CURRENT_VECTOR_BATCH_SOURCE,
            "embedding GQL JSON-current vector batch procedure executes",
        )
    }

    pub(crate) fn execute_json_current_text_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        self.execute_json_batch_source(
            registry,
            cache,
            JSON_CURRENT_TEXT_BATCH_SOURCE,
            "embedding GQL JSON-current text batch procedure executes",
        )
    }

    pub(crate) fn gql_json_current_vector_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_json_current_vector_batch_query(registry, cache);
        precision_basis_points(
            self.json_batch_precision(&table),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_json_current_vector_batch_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_json_current_vector_batch_query(registry, cache);
        precision_basis_points(
            self.json_batch_current_precision(&table),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_json_current_vector_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let table = self.execute_json_current_vector_batch_query(registry, cache);
        self.json_target_hit_basis_points(self.json_batch_target_hit_count(&table))
    }

    pub(crate) fn gql_json_current_text_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_json_current_text_batch_query(registry, cache);
        precision_basis_points(
            self.json_batch_precision(&table),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_json_current_text_batch_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_json_current_text_batch_query(registry, cache);
        precision_basis_points(
            self.json_batch_current_precision(&table),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_json_current_text_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let table = self.execute_json_current_text_batch_query(registry, cache);
        self.json_target_hit_basis_points(self.json_batch_target_hit_count(&table))
    }

    fn execute_json_batch_source(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
        source: &str,
        expected: &'static str,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session.execute_source(source, registry).expect(expected) {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn json_batch_precision(&self, table: &BindingTable) -> usize {
        self.json_batch_nodes(table)
            .filter(|(query_index, node)| {
                let topic = self.queries.get(*query_index).map(|query| query.topic);
                topic.is_some_and(|topic| self.topics_by_node.get(node) == Some(&topic))
            })
            .count()
    }

    fn json_batch_current_precision(&self, table: &BindingTable) -> usize {
        self.json_batch_nodes(table)
            .filter(|(query_index, node)| {
                let topic = self.queries.get(*query_index).map(|query| query.topic);
                topic.is_some_and(|topic| self.topics_by_node.get(node) == Some(&topic))
                    && self.current_by_node.get(node).copied().unwrap_or(false)
            })
            .count()
    }

    fn json_batch_target_hit_count(&self, table: &BindingTable) -> usize {
        let mut hits = vec![false; self.queries.len()];
        for (query_index, node) in self.json_batch_nodes(table) {
            let Some(expected) = self
                .queries
                .get(query_index)
                .and_then(|query| query.target_key)
            else {
                continue;
            };
            if self.target_by_node.get(&node) == Some(&expected) {
                hits[query_index] = true;
            }
        }
        hits.into_iter().filter(|hit| *hit).count()
    }

    fn json_batch_nodes<'a>(
        &'a self,
        table: &'a BindingTable,
    ) -> impl Iterator<Item = (usize, selene_core::NodeId)> + 'a {
        let query_column = table
            .column_index(db_string("query_index"))
            .expect("query_index column exists");
        let node_column = table
            .column_index(db_string("node_id"))
            .expect("node_id column exists");
        table.iter().filter_map(move |row| {
            let (Some(Value::Uint(query_index)), Some(Value::NodeRef(node))) =
                (row.get(query_column), row.get(node_column))
            else {
                return None;
            };
            Some((usize::try_from(*query_index).ok()?, *node))
        })
    }

    fn json_target_hit_basis_points(&self, hits: usize) -> Option<usize> {
        let target_queries = self
            .queries
            .iter()
            .filter(|query| query.target_key.is_some())
            .count();
        (target_queries > 0).then(|| precision_basis_points(hits, target_queries))
    }
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
