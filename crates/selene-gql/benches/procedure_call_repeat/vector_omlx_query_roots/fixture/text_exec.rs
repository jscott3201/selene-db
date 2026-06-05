use std::sync::Arc;

use selene_core::Value;
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};

use super::{OmlxGqlQueryRootFixture, TOP_K, istr};

const QUERY_ROOT_TEXT_SCORE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index MATCH (root)-[:OmlxSupports]->(candidate:OmlxEmbeddingDoc) WITH collect_list(candidate) AS candidates CALL selene.text_score_nodes('OmlxEmbeddingDoc', 'body', $query_text, candidates, 4) YIELD node_id, score RETURN node_id, score";
const QUERY_ROOT_TEXT_SCORE_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) MATCH (root)-[:OmlxSupports]->(candidate:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, anchor.query_text AS query_text, collect_list(candidate) AS candidates GROUP BY anchor.query_index, anchor.query_text ORDER BY query_index WITH collect_list(query_text) AS queries, collect_list(candidates) AS candidate_sets CALL selene.text_score_nodes_batch('OmlxEmbeddingDoc', 'body', queries, candidate_sets, 4) YIELD query_index, node_id, score RETURN query_index, node_id, score";

impl OmlxGqlQueryRootFixture {
    pub(crate) fn warm_query_root_text_score_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_text_score_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_text_score_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_text_score_batch_query(registry, Some(cache));
    }

    pub(crate) fn warm_query_root_text_score_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        self.execute_text_score_query_in_session(session, 0, registry);
    }

    pub(crate) fn execute_all_text_score_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_text_score_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_text_score_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_text_score_query_in_session(session, query_index, registry)
                    .row_count()
            })
            .sum()
    }

    pub(crate) fn gql_text_score_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table = self.execute_text_score_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.text_score_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_text_score_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_text_score_batch_query(registry, cache);
        precision_basis_points(
            self.text_score_batch_precision(&table),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn execute_text_score_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ROOT_TEXT_SCORE_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched query-root text scoring procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_text_score_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        self.execute_text_score_query_in_session(&mut session, query_index, registry)
    }

    fn execute_text_score_query_in_session(
        &self,
        session: &mut Session<'_>,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
    ) -> BindingTable {
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        session.bind_parameter(istr("query_text"), Value::String(istr(query.text)));
        match session
            .execute_source(QUERY_ROOT_TEXT_SCORE_SOURCE, registry)
            .expect("oMLX GQL query-root text scoring procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn text_score_precision(&self, topic: super::Topic, table: &BindingTable) -> usize {
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match row.get(node_column) {
                Some(Value::NodeRef(node)) => Some(*node),
                _ => None,
            })
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
            })
            .count()
    }

    fn text_score_batch_precision(&self, table: &BindingTable) -> usize {
        let query_column = table
            .column_index(istr("query_index"))
            .expect("query_index column exists");
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match (row.get(query_column), row.get(node_column)) {
                (Some(Value::Uint(query_index)), Some(Value::NodeRef(node))) => {
                    let query_index = usize::try_from(*query_index).ok()?;
                    let topic = self.queries.get(query_index)?.topic;
                    Some((topic, *node))
                }
                _ => None,
            })
            .filter(|(topic, node)| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| hit_topic == topic)
            })
            .count()
    }
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
