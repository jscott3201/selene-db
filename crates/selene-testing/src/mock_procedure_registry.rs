//! Mock procedure registry fixtures for analyzer tests.

use std::collections::HashMap;

use selene_core::{IStr, intern};
use selene_gql::{
    GqlType, ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry, ProcedureResult,
    ProcedureSignature, ProcedureTier, Value, procedure_registry::ProcedureError,
};

/// Test registry implementing selene-gql's planner-facing procedure boundary.
#[derive(Debug, Default)]
pub struct MockProcedureRegistry {
    procedures: HashMap<Box<[IStr]>, ProcedureMetadata>,
    next_handle: u64,
}

impl MockProcedureRegistry {
    /// Create an empty mock registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            procedures: HashMap::new(),
            next_handle: 1,
        }
    }

    /// Register a procedure and return the updated registry.
    #[must_use]
    pub fn with_procedure(
        mut self,
        name: Vec<IStr>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
    ) -> Self {
        self.insert_procedure(name, parameters, output_columns);
        self
    }

    /// Register a procedure in place.
    pub fn insert_procedure(
        &mut self,
        name: Vec<IStr>,
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
        name: Vec<IStr>,
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
        name: Vec<IStr>,
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
        name: Vec<IStr>,
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
        name: Vec<IStr>,
        parameters: Vec<ProcedureParameter>,
        output_columns: Vec<ProcedureOutputColumn>,
        mutability: ProcedureMutability,
        tier: ProcedureTier,
    ) {
        let handle = ProcedureHandle::new(self.next_handle);
        self.next_handle += 1;
        self.procedures.insert(
            name.into_boxed_slice(),
            ProcedureMetadata {
                handle,
                signature: ProcedureSignature { parameters },
                output_schema: ProcedureOutputSchema {
                    columns: output_columns,
                },
                tier,
                mutability,
                capability_required: None,
            },
        );
    }
}

impl ProcedureRegistry for MockProcedureRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.procedures.get(name).cloned()
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut selene_gql::ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        Err(ProcedureError::UnknownProcedure { name: Box::new([]) })
    }
}

const fn tier_for_mutability(mutability: ProcedureMutability) -> ProcedureTier {
    match mutability {
        ProcedureMutability::Read => ProcedureTier::Graph,
        ProcedureMutability::GraphWrite
        | ProcedureMutability::SchemaWrite
        | ProcedureMutability::Admin => ProcedureTier::Mutation,
    }
}

/// Registry containing every named procedure referenced by the default corpus.
///
/// M5a corpus cases are parse-oriented, so these signatures are intentionally
/// minimal. They let BRIEF-23's analyzer existence check run over the corpus
/// without making selene-testing depend on selene-pack.
#[must_use]
pub fn default_corpus_registry() -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure(
        vec![interned("selene"), interned("labels")],
        Vec::new(),
        vec![ProcedureOutputColumn {
            name: interned("label"),
            ty: GqlType::String,
        }],
    )
}

fn interned(value: &str) -> IStr {
    intern(value).expect("test fixture strings fit the interner")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_registry_lookup_round_trips() {
        let name = vec![interned("pkg"), interned("proc")];
        let registry = MockProcedureRegistry::new().with_procedure(
            name.clone(),
            vec![ProcedureParameter {
                name: interned("arg"),
                ty: GqlType::String,
                nullable: false,
            }],
            vec![ProcedureOutputColumn {
                name: interned("out"),
                ty: GqlType::Boolean,
            }],
        );

        let metadata = registry.lookup(&name).expect("procedure registered");
        assert_eq!(metadata.handle.raw(), 1);
        assert_eq!(metadata.signature.parameters.len(), 1);
        assert_eq!(metadata.output_schema.columns.len(), 1);
    }

    #[test]
    fn mock_registry_unknown_returns_none() {
        let registry = MockProcedureRegistry::new();
        assert!(
            registry
                .lookup(&[interned("missing"), interned("proc")])
                .is_none()
        );
    }
}
