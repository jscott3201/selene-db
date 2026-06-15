use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
};

use rustc_hash::{FxHashMap, FxHashSet};
use selene_core::{DbString, Value};
use selene_graph::{IndexProvider, SeleneGraph, SharedGraph, WriteTxn};

use crate::{ProcedureRegistry, plan::ImplDefinedCaps, runtime::BindingTableRegistry};

use super::{AdaptiveOptimizer, TxContext};

static EMPTY_PARAMETERS: BTreeMap<DbString, Value> = BTreeMap::new();

struct TxContextParts<'a, 'g> {
    snapshot: Arc<SeleneGraph>,
    impl_defined_caps: &'a ImplDefinedCaps,
    registry: &'a dyn ProcedureRegistry,
    providers: &'a [Arc<dyn IndexProvider>],
    parameters: Cow<'a, BTreeMap<DbString, Value>>,
    binding_tables: Rc<BindingTableRegistry>,
    reopt_hook: Option<&'a dyn AdaptiveOptimizer>,
    write_txn: Option<&'a mut WriteTxn<'g>>,
    maintenance_graph: Option<&'g SharedGraph>,
}

impl<'a, 'g> TxContext<'a, 'g> {
    fn base_parts(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: Cow<'a, BTreeMap<DbString, Value>>,
        binding_tables: Rc<BindingTableRegistry>,
    ) -> TxContextParts<'a, 'g> {
        TxContextParts {
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            binding_tables,
            reopt_hook: None,
            write_txn: None,
            maintenance_graph: None,
        }
    }

    fn from_parts(parts: TxContextParts<'a, 'g>) -> Self {
        Self {
            snapshot: parts.snapshot,
            impl_defined_caps: parts.impl_defined_caps,
            registry: parts.registry,
            providers: parts.providers,
            parameters: parts.parameters,
            binding_tables: parts.binding_tables,
            reopt_hook: parts.reopt_hook,
            plan_expr_ids: None,
            plan_subqueries: None,
            cancellation: None,
            deadline: None,
            row_cap: None,
            warning_sink: None,
            emitted_warnings: RefCell::new(FxHashSet::default()),
            result_rows_emitted: Cell::new(0),
            write_txn: parts.write_txn,
            maintenance_graph: parts.maintenance_graph,
            session_time_zone: jiff::tz::TimeZone::UTC,
            request_timestamp: jiff::Timestamp::now(),
            subquery_target_schema: RefCell::new(FxHashMap::default()),
        }
    }

    /// Construct a read-only context over an immutable graph snapshot.
    #[must_use]
    pub fn read_only(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
    ) -> Self {
        Self::read_only_with_parameters(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn read_only_with_parameters(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: &'a BTreeMap<DbString, Value>,
    ) -> Self {
        Self::from_parts(Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            Cow::Borrowed(parameters),
            Rc::new(BindingTableRegistry::new()),
        ))
    }

    /// Construct a read-only context carrying a future adaptive optimizer hook.
    #[must_use]
    pub fn read_only_with_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        reopt_hook: &'a dyn AdaptiveOptimizer,
    ) -> Self {
        Self::read_only_with_parameters_and_reopt(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            reopt_hook,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn read_only_with_parameters_and_reopt(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        reopt_hook: &'a dyn AdaptiveOptimizer,
        parameters: &'a BTreeMap<DbString, Value>,
    ) -> Self {
        let mut parts = Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            Cow::Borrowed(parameters),
            Rc::new(BindingTableRegistry::new()),
        );
        parts.reopt_hook = Some(reopt_hook);
        Self::from_parts(parts)
    }

    /// Construct a write-capable context over a graph write transaction.
    #[must_use]
    pub fn write(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        txn: &'a mut WriteTxn<'g>,
        providers: &'a [Arc<dyn IndexProvider>],
    ) -> Self {
        Self::write_with_parameters(
            snapshot,
            impl_defined_caps,
            registry,
            txn,
            providers,
            &EMPTY_PARAMETERS,
        )
    }

    pub(crate) fn write_with_parameters(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        txn: &'a mut WriteTxn<'g>,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: &'a BTreeMap<DbString, Value>,
    ) -> Self {
        let mut parts = Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            Cow::Borrowed(parameters),
            Rc::new(BindingTableRegistry::new()),
        );
        parts.write_txn = Some(txn);
        Self::from_parts(parts)
    }

    pub(crate) fn read_only_with_owned_parameters_and_registry(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: Cow<'a, BTreeMap<DbString, Value>>,
        binding_tables: Rc<BindingTableRegistry>,
    ) -> Self {
        Self::from_parts(Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            binding_tables,
        ))
    }

    pub(crate) fn write_with_owned_parameters_and_registry(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        txn: &'a mut WriteTxn<'g>,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: Cow<'a, BTreeMap<DbString, Value>>,
        binding_tables: Rc<BindingTableRegistry>,
    ) -> Self {
        let mut parts = Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            binding_tables,
        );
        parts.write_txn = Some(txn);
        Self::from_parts(parts)
    }

    pub(crate) fn maintenance_with_owned_parameters_and_registry(
        snapshot: Arc<SeleneGraph>,
        impl_defined_caps: &'a ImplDefinedCaps,
        registry: &'a dyn ProcedureRegistry,
        graph: &'g SharedGraph,
        providers: &'a [Arc<dyn IndexProvider>],
        parameters: Cow<'a, BTreeMap<DbString, Value>>,
        binding_tables: Rc<BindingTableRegistry>,
    ) -> Self {
        let mut parts = Self::base_parts(
            snapshot,
            impl_defined_caps,
            registry,
            providers,
            parameters,
            binding_tables,
        );
        parts.maintenance_graph = Some(graph);
        Self::from_parts(parts)
    }
}
