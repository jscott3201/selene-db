use std::sync::Arc;

use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};

use super::OmlxGqlQueryRootFixture;

impl OmlxGqlQueryRootFixture {
    pub(crate) fn gql_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let hits = (0..self.query_count())
            .map(|query_index| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.target_hit_count(query_index, &table)
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    pub(crate) fn gql_state_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let hits = (0..self.query_count())
            .map(|query_index| {
                let table =
                    self.execute_state_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.target_hit_count(query_index, &table)
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    pub(crate) fn gql_current_state_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let hits = (0..self.query_count())
            .map(|query_index| {
                let table = self.execute_current_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.target_hit_count(query_index, &table)
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    pub(crate) fn gql_provenance_state_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let hits = (0..self.query_count())
            .map(|query_index| {
                let table = self.execute_provenance_state_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                self.target_hit_count(query_index, &table)
            })
            .sum();
        self.target_hit_basis_points(hits)
    }

    pub(crate) fn gql_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let table = self.execute_batch_query(registry, cache);
        self.target_hit_basis_points(self.batch_target_hit_count(&table))
    }

    pub(crate) fn gql_current_state_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let table = self.execute_current_state_batch_query(registry, cache);
        self.target_hit_basis_points(self.batch_target_hit_count(&table))
    }

    pub(crate) fn gql_provenance_state_batch_target_hit_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> Option<usize> {
        let table = self.execute_provenance_state_batch_query(registry, cache);
        self.target_hit_basis_points(self.batch_target_hit_count(&table))
    }
}
