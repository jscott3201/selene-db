use std::sync::Arc;

use selene_core::Value;
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_testing::local_omlx::Topic;

use super::{OmlxGqlQueryRootFixture, TOP_K, istr};

#[path = "query_exec/shape.rs"]
mod shape;

const QUERY_ROOT_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_expanded_candidates('embedding', $query, roots, 'OmlxSupports', 4, 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_STATE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'omlx_support_facts', roots, 'OmlxSupports', 4, 'intersection', 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_CURRENT_STATE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'omlx_current_support_facts', roots, 'OmlxSupports', 4, 'intersection', 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_PROVENANCE_STATE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'omlx_provenance_current_support_facts', roots, 'OmlxSupports', 4, 'intersection', 'outgoing', 'cosine') YIELD node_id, distance RETURN node_id, distance";
const QUERY_ROOT_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, anchor.query AS query, collect_list(root) AS roots GROUP BY anchor.query_index, anchor.query ORDER BY query_index WITH collect_list(query) AS queries, collect_list(roots) AS root_sets CALL selene.vector_score_expanded_candidates_batch('embedding', queries, root_sets, 'OmlxSupports', 4, 'outgoing', 'cosine') YIELD query_index, node_id, distance RETURN query_index, node_id, distance";

impl OmlxGqlQueryRootFixture {
    pub(crate) fn warm_query_root_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_state_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_state_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_current_state_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_current_state_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_provenance_state_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_provenance_state_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_batch_query(registry, Some(cache));
    }

    pub(crate) fn warm_query_root_expansion_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        self.execute_query_source_in_session(
            session,
            QUERY_ROOT_SOURCE,
            0,
            registry,
            "oMLX GQL query-root vector procedure executes",
        );
    }

    pub(crate) fn warm_query_root_state_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        self.execute_query_source_in_session(
            session,
            QUERY_ROOT_STATE_SOURCE,
            0,
            registry,
            "oMLX GQL query-root state vector procedure executes",
        );
    }

    pub(crate) fn warm_query_root_current_state_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        self.execute_query_source_in_session(
            session,
            QUERY_ROOT_CURRENT_STATE_SOURCE,
            0,
            registry,
            "oMLX GQL query-root current-state vector procedure executes",
        );
    }

    pub(crate) fn warm_query_root_provenance_state_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        self.execute_query_source_in_session(
            session,
            QUERY_ROOT_PROVENANCE_STATE_SOURCE,
            0,
            registry,
            "oMLX GQL query-root provenance-state vector procedure executes",
        );
    }

    pub(crate) fn execute_all_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query_source_in_session(
                    session,
                    QUERY_ROOT_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL query-root vector procedure executes",
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_state_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_state_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_state_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query_source_in_session(
                    session,
                    QUERY_ROOT_STATE_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL query-root state vector procedure executes",
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_current_state_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_current_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_current_state_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query_source_in_session(
                    session,
                    QUERY_ROOT_CURRENT_STATE_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL query-root current-state vector procedure executes",
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_provenance_state_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_provenance_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_provenance_state_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query_source_in_session(
                    session,
                    QUERY_ROOT_PROVENANCE_STATE_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL query-root provenance-state vector procedure executes",
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn gql_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.current_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_state_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_state_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_current_state_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table = self.execute_current_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.current_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_provenance_state_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table = self.execute_provenance_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.current_precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    pub(crate) fn gql_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let table = self.execute_batch_query(registry, cache);
        precision_basis_points(self.batch_precision(&table), self.query_count() * TOP_K)
    }

    fn execute_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session
            .execute_source(QUERY_ROOT_SOURCE, registry)
            .expect("oMLX GQL query-root vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_state_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        self.execute_state_source_query(
            query_index,
            registry,
            cache,
            QUERY_ROOT_STATE_SOURCE,
            "oMLX GQL query-root state vector procedure executes",
        )
    }

    fn execute_current_state_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        self.execute_state_source_query(
            query_index,
            registry,
            cache,
            QUERY_ROOT_CURRENT_STATE_SOURCE,
            "oMLX GQL query-root current-state vector procedure executes",
        )
    }

    fn execute_provenance_state_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        self.execute_state_source_query(
            query_index,
            registry,
            cache,
            QUERY_ROOT_PROVENANCE_STATE_SOURCE,
            "oMLX GQL query-root provenance-state vector procedure executes",
        )
    }

    fn execute_state_source_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
        source: &str,
        expected: &'static str,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session.execute_source(source, registry).expect(expected) {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_query_source_in_session(
        &self,
        session: &mut Session<'_>,
        source: &str,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        expected: &'static str,
    ) -> BindingTable {
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session.execute_source(source, registry).expect(expected) {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    pub(crate) fn execute_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ROOT_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched query-root vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn precision(&self, topic: Topic, table: &BindingTable) -> usize {
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

    fn current_precision(&self, topic: Topic, table: &BindingTable) -> usize {
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
                    && self.current_by_node.get(node).copied().unwrap_or(false)
            })
            .count()
    }

    fn batch_precision(&self, table: &BindingTable) -> usize {
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
