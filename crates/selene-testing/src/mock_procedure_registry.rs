//! Mock procedure registry fixtures for analyzer tests.

use std::collections::HashMap;

use selene_core::{DbString, db_string};
use selene_gql::{
    GqlType, ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry, ProcedureResult,
    ProcedureSignature, ProcedureTier, Value, procedure_registry::ProcedureError,
};

/// Test registry implementing selene-gql's planner-facing procedure boundary.
#[derive(Debug, Default)]
pub struct MockProcedureRegistry {
    procedures: HashMap<Box<[DbString]>, ProcedureMetadata>,
    results: HashMap<ProcedureHandle, ProcedureResult>,
    next_handle: u64,
    version: u64,
}

impl MockProcedureRegistry {
    /// Create an empty mock registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            procedures: HashMap::new(),
            results: HashMap::new(),
            next_handle: 1,
            version: 0,
        }
    }

    /// Register a procedure and return the updated registry.
    #[must_use]
    pub fn with_procedure(
        mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
    ) -> Self {
        self.insert_procedure(name, parameters, output_columns);
        self
    }

    /// Register a procedure in place.
    pub fn insert_procedure(
        &mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
    ) {
        self.insert_procedure_with_mutability(
            name,
            parameters,
            output_columns,
            ProcedureMutability::Read,
        );
    }

    /// Register a procedure with a specific mutability and return the updated
    /// registry.
    #[must_use]
    pub fn with_procedure_mutability(
        mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
        mutability: ProcedureMutability,
    ) -> Self {
        self.insert_procedure_with_mutability(name, parameters, output_columns, mutability);
        self
    }

    /// Register a procedure with explicit mutability and tier metadata.
    #[must_use]
    pub fn with_procedure_tier(
        mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
        mutability: ProcedureMutability,
        tier: ProcedureTier,
    ) -> Self {
        self.insert_procedure_with_tier(name, parameters, output_columns, mutability, tier);
        self
    }

    /// Register a procedure with a specific mutability in place.
    pub fn insert_procedure_with_mutability(
        &mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
        mutability: ProcedureMutability,
    ) {
        self.insert_procedure_with_tier(
            name,
            parameters,
            output_columns,
            mutability,
            tier_for_mutability(mutability),
        );
    }

    /// Register a procedure with explicit mutability and tier metadata in place.
    pub fn insert_procedure_with_tier(
        &mut self,
        name: Vec<DbString>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
        mutability: ProcedureMutability,
        tier: ProcedureTier,
    ) {
        let handle = ProcedureHandle::new(self.next_handle);
        self.next_handle += 1;
        self.version = self.version.saturating_add(1);
        self.procedures.insert(
            name.into_boxed_slice(),
            ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(parameters),
                ProcedureOutputSchema {
                    columns: output_columns,
                },
                tier,
                mutability,
            ),
        );
    }

    /// Register a deterministic runtime result for a procedure handle.
    #[must_use]
    pub fn with_result(mut self, handle: ProcedureHandle, result: ProcedureResult) -> Self {
        self.insert_result(handle, result);
        self
    }

    /// Register a deterministic runtime result in place.
    pub fn insert_result(&mut self, handle: ProcedureHandle, result: ProcedureResult) {
        self.version = self.version.saturating_add(1);
        self.results.insert(handle, result);
    }

    /// Return the deterministic runtime result registered for a procedure handle.
    #[must_use]
    pub fn result_for(&self, handle: ProcedureHandle) -> Option<&ProcedureResult> {
        self.results.get(&handle)
    }
}

impl ProcedureRegistry for MockProcedureRegistry {
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
        self.procedures.get(name).cloned()
    }

    fn registry_version(&self) -> u64 {
        self.version
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut selene_gql::ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        self.results
            .get(&handle)
            .cloned()
            .ok_or(ProcedureError::UnknownProcedure { name: Box::new([]) })
    }
}

const fn tier_for_mutability(mutability: ProcedureMutability) -> ProcedureTier {
    match mutability {
        ProcedureMutability::Read => ProcedureTier::Graph,
        ProcedureMutability::SchemaWrite => ProcedureTier::Mutation,
        ProcedureMutability::MaintenanceWrite => ProcedureTier::Maintenance,
    }
}

/// Registry containing every named procedure referenced by the default corpus.
///
/// Corpus cases are parse-oriented, so these signatures are intentionally
/// minimal. They let analyzer existence checks run over the corpus without
/// making selene-testing depend on selene-gql.
#[must_use]
pub fn default_corpus_registry() -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure(
        vec![dbs("selene"), dbs("labels")],
        Vec::new(),
        vec![ProcedureOutputColumn::new(dbs("label"), GqlType::String)],
    )
}

fn dbs(value: &str) -> DbString {
    db_string(value).expect("test fixture strings fit DB string cap")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_registry_lookup_round_trips() {
        let name = vec![dbs("pkg"), dbs("proc")];
        let registry = MockProcedureRegistry::new().with_procedure(
            name.clone(),
            vec![ProcedureParameter::new(dbs("arg"), GqlType::String, false)],
            vec![ProcedureOutputColumn::new(dbs("out"), GqlType::Boolean)],
        );

        let metadata = registry.lookup(&name).expect("procedure registered");
        assert_eq!(metadata.handle.raw(), 1);
        assert_eq!(metadata.signature.parameters.len(), 1);
        assert_eq!(metadata.output_schema.columns.len(), 1);
    }

    #[test]
    fn mock_registry_unknown_returns_none() {
        let registry = MockProcedureRegistry::new();
        assert!(registry.lookup(&[dbs("missing"), dbs("proc")]).is_none());
    }
}
