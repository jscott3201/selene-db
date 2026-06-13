use std::sync::Arc;

use selene_core::{NodeId, Value};
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};

use super::{OmlxGqlQueryRootFixture, TOP_K, db_string};

const CURRENT_STATE_TEXT_VECTOR_RRF_SOURCE: &str =
    "CALL selene.reciprocal_rank_fusion($rankings, 4) YIELD node_id, score RETURN node_id, score";

impl OmlxGqlQueryRootFixture {
    pub(crate) fn warm_query_root_current_state_text_vector_rrf_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_current_state_text_vector_rrf_batch(registry, Some(cache));
    }

    pub(crate) fn gql_current_state_text_vector_rrf_batch_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let rankings = self.execute_current_state_text_vector_rrf_batch(registry, cache);
        precision_basis_points(
            self.ranked_topic_precision(&rankings),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_current_state_text_vector_rrf_batch_current_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let rankings = self.execute_current_state_text_vector_rrf_batch(registry, cache);
        precision_basis_points(
            self.ranked_current_precision(&rankings),
            self.query_count() * TOP_K,
        )
    }

    pub(crate) fn gql_current_state_text_vector_rrf_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let rankings = self.execute_current_state_text_vector_rrf_batch(registry, cache);
        self.target_hit_basis_points(self.ranked_target_hit_count(&rankings))
    }

    pub(crate) fn execute_current_state_text_vector_rrf_batch(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Vec<Vec<NodeId>> {
        let text_table = self
            .execute_current_state_text_score_batch_query(registry, cache.as_ref().map(Arc::clone));
        let vector_table =
            self.execute_current_state_batch_query(registry, cache.as_ref().map(Arc::clone));
        let text_rankings = self.batch_rankings(&text_table);
        let vector_rankings = self.batch_rankings(&vector_table);
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        text_rankings
            .iter()
            .zip(vector_rankings.iter())
            .map(|(text, vector)| {
                let rankings = Value::List(vec![node_list_value(text), node_list_value(vector)]);
                session.bind_parameter(db_string("rankings"), rankings);
                match session
                    .execute_source(CURRENT_STATE_TEXT_VECTOR_RRF_SOURCE, registry)
                    .expect("oMLX GQL current-state text/vector RRF procedure executes")
                {
                    StatementOutput::Rows(table) => node_rankings(&table),
                    other => panic!("unexpected output: {other:?}"),
                }
            })
            .collect()
    }

    fn batch_rankings(&self, table: &BindingTable) -> Vec<Vec<NodeId>> {
        let query_column = table
            .column_index(db_string("query_index"))
            .expect("query_index column exists");
        let node_column = table
            .column_index(db_string("node_id"))
            .expect("node_id column exists");
        let mut rankings = vec![Vec::new(); self.query_count()];
        for row in table.iter() {
            let (Some(Value::Uint(query_index)), Some(Value::NodeRef(node))) =
                (row.get(query_column), row.get(node_column))
            else {
                continue;
            };
            let Ok(query_index) = usize::try_from(*query_index) else {
                continue;
            };
            if let Some(ranking) = rankings.get_mut(query_index) {
                ranking.push(*node);
            }
        }
        rankings
    }

    fn ranked_topic_precision(&self, rankings: &[Vec<NodeId>]) -> usize {
        rankings
            .iter()
            .enumerate()
            .map(|(query_index, ranking)| {
                let topic = self
                    .queries
                    .get(query_index)
                    .expect("query index matches rankings")
                    .topic;
                ranking
                    .iter()
                    .filter(|node| {
                        self.topics_by_node
                            .get(node)
                            .is_some_and(|hit_topic| *hit_topic == topic)
                    })
                    .count()
            })
            .sum()
    }

    fn ranked_current_precision(&self, rankings: &[Vec<NodeId>]) -> usize {
        rankings
            .iter()
            .enumerate()
            .map(|(query_index, ranking)| {
                let topic = self
                    .queries
                    .get(query_index)
                    .expect("query index matches rankings")
                    .topic;
                ranking
                    .iter()
                    .filter(|node| {
                        self.topics_by_node
                            .get(node)
                            .is_some_and(|hit_topic| *hit_topic == topic)
                            && self.current_by_node.get(node).copied().unwrap_or(false)
                    })
                    .count()
            })
            .sum()
    }

    fn ranked_target_hit_count(&self, rankings: &[Vec<NodeId>]) -> usize {
        rankings
            .iter()
            .enumerate()
            .map(|(query_index, ranking)| {
                let Some(expected) = self
                    .queries
                    .get(query_index)
                    .and_then(|query| query.target_key)
                else {
                    return 0;
                };
                ranking
                    .iter()
                    .any(|node| self.target_by_node.get(node) == Some(&expected))
                    as usize
            })
            .sum()
    }
}

fn node_list_value(nodes: &[NodeId]) -> Value {
    Value::List(nodes.iter().copied().map(Value::NodeRef).collect())
}

fn node_rankings(table: &BindingTable) -> Vec<NodeId> {
    let node_column = table
        .column_index(db_string("node_id"))
        .expect("node_id column exists");
    table
        .iter()
        .filter_map(|row| match row.get(node_column) {
            Some(Value::NodeRef(node)) => Some(*node),
            _ => None,
        })
        .collect()
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
