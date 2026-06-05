use std::{num::NonZeroUsize, sync::Arc};

use selene_core::Value;
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};

use super::super::{OmlxGqlQueryRootFixture, istr};

const QUERY_ANCHOR_LOOKUP_SOURCE: &str =
    "MATCH (anchor:OmlxQueryAnchor) WHERE anchor.query_index = $query_index RETURN anchor";
const QUERY_ANCHOR_LOOKUP_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor) WITH anchor.query_index AS query_index, anchor AS anchor ORDER BY query_index RETURN query_index, anchor";
const QUERY_ROOT_ROWS_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index RETURN root";
const QUERY_ROOT_ROWS_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, root AS root ORDER BY query_index RETURN query_index, root";
const QUERY_ROOT_MATERIALIZE_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots RETURN roots";
const QUERY_ROOT_MATERIALIZE_BATCH_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WITH anchor.query_index AS query_index, collect_list(root) AS roots GROUP BY anchor.query_index ORDER BY query_index RETURN query_index, roots";
const REUSED_SESSION_PLAN_CACHE_CAPACITY: usize = 8;

impl OmlxGqlQueryRootFixture {
    pub(crate) fn reusable_session(&self) -> Session<'_> {
        Session::new(&self.graph)
    }

    pub(crate) fn reusable_plan_cache_session(&self) -> Session<'_> {
        Session::new(&self.graph).with_plan_cache(
            NonZeroUsize::new(REUSED_SESSION_PLAN_CACHE_CAPACITY).expect("nonzero"),
        )
    }

    pub(crate) fn warm_anchor_lookup_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        execute_query_index_source_in_session(
            session,
            QUERY_ANCHOR_LOOKUP_SOURCE,
            0,
            registry,
            "oMLX GQL query anchor lookup executes",
        );
    }

    pub(crate) fn warm_root_materialize_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) {
        execute_query_index_source_in_session(
            session,
            QUERY_ROOT_MATERIALIZE_SOURCE,
            0,
            registry,
            "oMLX GQL root materialization executes",
        );
    }

    pub(crate) fn warm_query_anchor_lookup_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_anchor_lookup_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_anchor_lookup_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_anchor_lookup_batch_query(registry, Some(cache));
    }

    pub(crate) fn warm_query_root_rows_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_root_rows_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_rows_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_root_rows_batch_query(registry, Some(cache));
    }

    pub(crate) fn warm_query_root_materialize_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_root_materialize_query(0, registry, Some(cache));
    }

    pub(crate) fn warm_query_root_materialize_batch_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_root_materialize_batch_query(registry, Some(cache));
    }

    pub(crate) fn execute_all_anchor_lookup_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_anchor_lookup_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_all_anchor_lookup_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                execute_query_index_source_in_session(
                    session,
                    QUERY_ANCHOR_LOOKUP_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL query anchor lookup executes",
                )
                .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_anchor_lookup_batch_count(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        self.execute_anchor_lookup_batch_query(registry, cache)
            .row_count()
    }

    pub(crate) fn execute_all_root_rows_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_root_rows_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    pub(crate) fn execute_root_rows_batch_count(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        self.execute_root_rows_batch_query(registry, cache)
            .row_count()
    }

    pub(crate) fn execute_all_root_materialize_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                let table = self.execute_root_materialize_query(
                    query_index,
                    registry,
                    cache.as_ref().map(Arc::clone),
                );
                root_count(&table)
            })
            .sum()
    }

    pub(crate) fn execute_all_root_materialize_queries_in_session(
        &self,
        session: &mut Session<'_>,
        registry: &BuiltinProcedureRegistry,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                let table = execute_query_index_source_in_session(
                    session,
                    QUERY_ROOT_MATERIALIZE_SOURCE,
                    query_index,
                    registry,
                    "oMLX GQL root materialization executes",
                );
                root_count(&table)
            })
            .sum()
    }

    pub(crate) fn execute_root_materialize_batch_count(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        root_count(&self.execute_root_materialize_batch_query(registry, cache))
    }

    fn execute_anchor_lookup_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        execute_query_index_source_in_session(
            &mut session,
            QUERY_ANCHOR_LOOKUP_SOURCE,
            query_index,
            registry,
            "oMLX GQL query anchor lookup executes",
        )
    }

    fn execute_anchor_lookup_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ANCHOR_LOOKUP_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched query anchor lookup executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_root_rows_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        execute_query_index_source_in_session(
            &mut session,
            QUERY_ROOT_ROWS_SOURCE,
            query_index,
            registry,
            "oMLX GQL root-row traversal executes",
        )
    }

    fn execute_root_rows_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ROOT_ROWS_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched root-row traversal executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn execute_root_materialize_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        execute_query_index_source_in_session(
            &mut session,
            QUERY_ROOT_MATERIALIZE_SOURCE,
            query_index,
            registry,
            "oMLX GQL root materialization executes",
        )
    }

    fn execute_root_materialize_batch_query(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        match session
            .execute_source(QUERY_ROOT_MATERIALIZE_BATCH_SOURCE, registry)
            .expect("oMLX GQL batched root materialization executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }
}

fn execute_query_index_source_in_session(
    session: &mut Session<'_>,
    source: &str,
    query_index: usize,
    registry: &BuiltinProcedureRegistry,
    expected: &'static str,
) -> BindingTable {
    session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
    match session.execute_source(source, registry).expect(expected) {
        StatementOutput::Rows(table) => table,
        other => panic!("unexpected output: {other:?}"),
    }
}

fn root_count(table: &BindingTable) -> usize {
    let roots_column = table
        .column_index(istr("roots"))
        .expect("roots column exists");
    table
        .iter()
        .map(|row| match row.get(roots_column) {
            Some(Value::List(roots)) => roots.len(),
            other => panic!("expected roots list, got {other:?}"),
        })
        .sum()
}
