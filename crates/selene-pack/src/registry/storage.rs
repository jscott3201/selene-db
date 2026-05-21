//! Registry storage and metadata conversion.

use std::{collections::HashMap, sync::Arc};

use papaya::HashMap as PapayaHashMap;
use selene_core::{IStr, intern_with_admission};
use selene_gql::{
    ProcedureHandle, ProcedureMetadata, ProcedureOutputColumn, ProcedureOutputSchema,
    ProcedureParameter, ProcedureSignature, ProcedureTier,
};

use crate::{
    builtin::{
        GraphProcedureBuiltIn, MutationProcedureBuiltIn, StaticOutputColumn, StaticParameter,
        UNSTABLE_BUILTIN_CONTENT_HASH,
    },
    error::RegistryError,
    external::{
        ExternalGraphProcedure, ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter,
    },
};

use selene_gql::ProcedureMutability;

pub(crate) type NameKey = Box<[IStr]>;

#[derive(Clone)]
pub(crate) enum TierEntry {
    Graph(Arc<dyn GraphProcedureBuiltIn>),
    Mutation(Arc<dyn MutationProcedureBuiltIn>),
    ExternalGraph(Arc<dyn ExternalGraphProcedure>),
    ExternalMutation(Arc<dyn ExternalMutationProcedure>),
}

impl std::fmt::Debug for TierEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TierEntry")
            .field("tier", &self.tier())
            .finish()
    }
}

impl TierEntry {
    pub(crate) fn tier(&self) -> ProcedureTier {
        match self {
            Self::Graph(_) | Self::ExternalGraph(_) => ProcedureTier::Graph,
            Self::Mutation(_) | Self::ExternalMutation(_) => ProcedureTier::Mutation,
        }
    }

    fn declared_tier(&self) -> ProcedureTier {
        match self {
            Self::Graph(procedure) => procedure.tier(),
            Self::Mutation(procedure) => procedure.tier(),
            Self::ExternalGraph(_) => ProcedureTier::Graph,
            Self::ExternalMutation(_) => ProcedureTier::Mutation,
        }
    }

    fn mutability(&self) -> ProcedureMutability {
        match self {
            Self::Graph(procedure) => procedure.mutability(),
            Self::Mutation(procedure) => procedure.mutability(),
            Self::ExternalGraph(_) => ProcedureMutability::Read,
            Self::ExternalMutation(_) => ProcedureMutability::GraphWrite,
        }
    }

    pub(crate) fn name(&self) -> &'static [&'static str] {
        match self {
            Self::Graph(procedure) => procedure.name(),
            Self::Mutation(procedure) => procedure.name(),
            Self::ExternalGraph(procedure) => procedure.name(),
            Self::ExternalMutation(procedure) => procedure.name(),
        }
    }

    fn content_hash(&self) -> [u8; 32] {
        match self {
            Self::Graph(procedure) => procedure.content_hash(),
            Self::Mutation(procedure) => procedure.content_hash(),
            Self::ExternalGraph(_) => UNSTABLE_BUILTIN_CONTENT_HASH,
            Self::ExternalMutation(_) => UNSTABLE_BUILTIN_CONTENT_HASH,
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Graph(procedure) => procedure.description(),
            Self::Mutation(procedure) => procedure.description(),
            Self::ExternalGraph(procedure) => procedure.description(),
            Self::ExternalMutation(procedure) => procedure.description(),
        }
    }

    fn since_version(&self) -> &'static str {
        match self {
            Self::Graph(procedure) => procedure.since_version(),
            Self::Mutation(procedure) => procedure.since_version(),
            Self::ExternalGraph(procedure) => procedure.since_version(),
            Self::ExternalMutation(procedure) => procedure.since_version(),
        }
    }
}

pub(crate) struct PendingEntry {
    handle: ProcedureHandle,
    attempted_tier: ProcedureTier,
    entry: TierEntry,
}

impl PendingEntry {
    pub(crate) fn graph(handle: ProcedureHandle, builtin: impl GraphProcedureBuiltIn) -> Self {
        Self {
            handle,
            attempted_tier: ProcedureTier::Graph,
            entry: TierEntry::Graph(Arc::new(builtin)),
        }
    }

    pub(crate) fn mutation(
        handle: ProcedureHandle,
        builtin: impl MutationProcedureBuiltIn,
    ) -> Self {
        Self {
            handle,
            attempted_tier: ProcedureTier::Mutation,
            entry: TierEntry::Mutation(Arc::new(builtin)),
        }
    }

    pub(crate) fn external_graph(
        handle: ProcedureHandle,
        procedure: Arc<dyn ExternalGraphProcedure>,
    ) -> Self {
        Self {
            handle,
            attempted_tier: ProcedureTier::Graph,
            entry: TierEntry::ExternalGraph(procedure),
        }
    }

    pub(crate) fn external_mutation(
        handle: ProcedureHandle,
        procedure: Arc<dyn ExternalMutationProcedure>,
    ) -> Self {
        Self {
            handle,
            attempted_tier: ProcedureTier::Mutation,
            entry: TierEntry::ExternalMutation(procedure),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RegistryStorage {
    by_name: PapayaHashMap<NameKey, ProcedureMetadata>,
    by_handle: PapayaHashMap<ProcedureHandle, TierEntry>,
}

impl RegistryStorage {
    pub(crate) fn empty() -> Self {
        Self {
            by_name: PapayaHashMap::new(),
            by_handle: PapayaHashMap::new(),
        }
    }

    pub(crate) fn from_pending(pending: Vec<PendingEntry>) -> Result<Self, RegistryError> {
        let mut staged = HashMap::<NameKey, StagedEntry>::new();
        let mut ordered = Vec::new();

        for pending_entry in pending {
            let name = intern_name(pending_entry.entry.name())?;
            validate_tier(
                &name,
                pending_entry.entry.mutability(),
                pending_entry.entry.declared_tier(),
                pending_entry.attempted_tier,
            )?;
            let hash = pending_entry.entry.content_hash();

            if let Some(existing) = staged.get(&name) {
                if existing.tier != pending_entry.attempted_tier {
                    return Err(RegistryError::TierMismatch {
                        name,
                        declared: existing.tier,
                        attempted: pending_entry.attempted_tier,
                    });
                }
                // Built-ins without manifest-derived hashes are not stable
                // enough for idempotent duplicate admission.
                let unstable_hash = existing.hash == UNSTABLE_BUILTIN_CONTENT_HASH
                    || hash == UNSTABLE_BUILTIN_CONTENT_HASH;
                if unstable_hash || existing.hash != hash {
                    return Err(RegistryError::Conflict {
                        name,
                        existing_hash: existing.hash,
                        new_hash: hash,
                    });
                }
                continue;
            }

            let procedure_metadata = procedure_metadata(&pending_entry)?;
            staged.insert(
                name.clone(),
                StagedEntry {
                    hash,
                    tier: pending_entry.attempted_tier,
                },
            );
            ordered.push((name, procedure_metadata, pending_entry));
        }

        let storage = Self::empty();
        {
            let name_map = storage.by_name.pin();
            let handle_map = storage.by_handle.pin();
            for (name, metadata, pending_entry) in ordered {
                name_map.insert(name, metadata);
                handle_map.insert(pending_entry.handle, pending_entry.entry);
            }
        }
        Ok(storage)
    }

    pub(crate) fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.by_name.pin().get(name).cloned()
    }

    pub(crate) fn iter_handles(&self) -> Vec<(Vec<IStr>, ProcedureMetadata)> {
        self.by_name
            .pin()
            .iter()
            .map(|(name, metadata)| (name.to_vec(), metadata.clone()))
            .collect()
    }

    pub(crate) fn entry(&self, handle: ProcedureHandle) -> Option<TierEntry> {
        self.by_handle.pin().get(&handle).cloned()
    }
}

#[derive(Clone, Copy)]
struct StagedEntry {
    hash: [u8; 32],
    tier: ProcedureTier,
}

fn validate_tier(
    name: &NameKey,
    mutability: ProcedureMutability,
    declared: ProcedureTier,
    attempted: ProcedureTier,
) -> Result<(), RegistryError> {
    if declared == ProcedureTier::Persist {
        return Err(RegistryError::PersistTierNotInV1 { name: name.clone() });
    }
    if declared != attempted {
        return Err(RegistryError::TierMismatch {
            name: name.clone(),
            declared,
            attempted,
        });
    }
    // Mirror selene_gql::runtime::pipeline::call::context::tier_for_mutability:
    // a read-mutability builtin must declare Graph (or Persist, rejected above)
    // tier; any write mutability must declare Mutation. Catching this drift at
    // construction prevents a tier mismatch from surfacing as a runtime error.
    let expected_for_mutability = match mutability {
        ProcedureMutability::Read => ProcedureTier::Graph,
        ProcedureMutability::GraphWrite
        | ProcedureMutability::SchemaWrite
        | ProcedureMutability::Admin => ProcedureTier::Mutation,
    };
    if declared != expected_for_mutability {
        return Err(RegistryError::MutabilityTierMismatch {
            name: name.clone(),
            declared_tier: declared,
            declared_mutability: mutability,
            expected_tier: expected_for_mutability,
        });
    }
    Ok(())
}

fn procedure_metadata(pending: &PendingEntry) -> Result<ProcedureMetadata, RegistryError> {
    let (parameters, columns) = match &pending.entry {
        TierEntry::Graph(procedure) => (
            procedure
                .signature_static()
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>, _>>()?,
            procedure
                .output_columns_static()
                .iter()
                .map(output_column)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        TierEntry::Mutation(procedure) => (
            procedure
                .signature_static()
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>, _>>()?,
            procedure
                .output_columns_static()
                .iter()
                .map(output_column)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        TierEntry::ExternalGraph(procedure) => (
            procedure
                .signature()
                .iter()
                .map(external_parameter)
                .collect::<Result<Vec<_>, _>>()?,
            procedure
                .output_columns()
                .iter()
                .map(external_output_column)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        TierEntry::ExternalMutation(procedure) => (
            procedure
                .signature()
                .iter()
                .map(external_parameter)
                .collect::<Result<Vec<_>, _>>()?,
            procedure
                .output_columns()
                .iter()
                .map(external_output_column)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    Ok(ProcedureMetadata::new(
        pending.handle,
        ProcedureSignature::new(parameters).with_since_version(pending.entry.since_version()),
        ProcedureOutputSchema { columns },
        pending.attempted_tier,
        pending.entry.mutability(),
        None,
    )
    .with_description(pending.entry.description()))
}

fn parameter(parameter: &StaticParameter) -> Result<ProcedureParameter, RegistryError> {
    let mut result = ProcedureParameter::new(
        intern_static(parameter.name, "parameter")?,
        parameter.ty.clone(),
        parameter.nullable,
    )
    .with_description(parameter.description);
    if let Some(default_doc) = parameter.default_doc {
        result = result.with_default_doc(default_doc);
    }
    if let Some(default) = parameter.default {
        result = result.with_default(default);
    }
    Ok(result)
}

fn output_column(column: &StaticOutputColumn) -> Result<ProcedureOutputColumn, RegistryError> {
    Ok(ProcedureOutputColumn::new(
        intern_static(column.name, "output column")?,
        column.ty.clone(),
    )
    .with_description(column.description))
}

fn external_parameter(parameter: &ExternalParameter) -> Result<ProcedureParameter, RegistryError> {
    let mut result = ProcedureParameter::new(
        intern_static(parameter.name, "parameter")?,
        parameter.ty.clone(),
        parameter.nullable,
    )
    .with_description(parameter.description);
    if let Some(default_doc) = parameter.default_doc {
        result = result.with_default_doc(default_doc);
    }
    if let Some(default) = parameter.default {
        result = result.with_default(default);
    }
    Ok(result)
}

fn external_output_column(
    column: &ExternalOutputColumn,
) -> Result<ProcedureOutputColumn, RegistryError> {
    Ok(ProcedureOutputColumn::new(
        intern_static(column.name, "output column")?,
        column.ty.clone(),
    )
    .with_description(column.description))
}

fn intern_name(raw: &'static [&'static str]) -> Result<NameKey, RegistryError> {
    if raw.is_empty() {
        return Err(RegistryError::InvalidName {
            name: Box::new([]),
            reason: "procedure name must contain at least one segment",
        });
    }
    raw.iter()
        .map(|segment| {
            if segment.is_empty() {
                return Err(RegistryError::InvalidName {
                    name: raw
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                    reason: "procedure name segments must be non-empty",
                });
            }
            intern_static(segment, "procedure name")
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

fn intern_static(value: &str, kind: &'static str) -> Result<IStr, RegistryError> {
    intern_with_admission(value)
        .map(|(istr, _was_new)| istr)
        .map_err(|_source| RegistryError::InternerCapExhausted {
            detail: format!("{kind} '{value}'"),
        })
}

#[cfg(test)]
mod tests {
    use selene_gql::{GqlType, ProcedureDefaultValue};

    use super::*;

    #[test]
    fn static_parameter_default_round_trips() {
        let parameter = StaticParameter::new("enabled", GqlType::Boolean, false)
            .with_description("Whether the option is enabled.")
            .with_default_doc("FALSE")
            .with_default(ProcedureDefaultValue::Boolean(false));

        let converted = super::parameter(&parameter).expect("static parameter converts");

        assert_eq!(converted.name.as_str(), "enabled");
        assert_eq!(converted.default_doc, Some("FALSE"));
        assert_eq!(
            converted.default,
            Some(ProcedureDefaultValue::Boolean(false))
        );
    }

    #[test]
    fn external_parameter_default_round_trips() {
        let parameter = ExternalParameter::new("limit", GqlType::Integer, false)
            .with_description("Optional limit.")
            .with_default_doc("10")
            .with_default(ProcedureDefaultValue::Integer(10));

        let converted = super::external_parameter(&parameter).expect("external parameter converts");

        assert_eq!(converted.name.as_str(), "limit");
        assert_eq!(converted.default_doc, Some("10"));
        assert_eq!(converted.default, Some(ProcedureDefaultValue::Integer(10)));
    }
}
