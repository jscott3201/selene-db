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
        BuiltInMetadata, GraphProcedureBuiltIn, MutationProcedureBuiltIn, StaticOutputColumn,
        StaticParameter,
    },
    error::RegistryError,
};

pub(crate) type NameKey = Box<[IStr]>;

#[derive(Clone)]
pub(crate) enum TierEntry {
    Graph(Arc<dyn GraphProcedureBuiltIn>),
    #[allow(dead_code)] // Reserved for BRIEF-42's first mutation-tier built-in.
    Mutation(Arc<dyn MutationProcedureBuiltIn>),
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
            Self::Graph(_) => ProcedureTier::Graph,
            Self::Mutation(_) => ProcedureTier::Mutation,
        }
    }

    fn metadata(&self) -> &dyn BuiltInMetadata {
        match self {
            Self::Graph(procedure) => procedure.as_ref(),
            Self::Mutation(procedure) => procedure.as_ref(),
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

    #[allow(dead_code)] // Reserved for BRIEF-42's first mutation-tier built-in.
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
            let metadata = pending_entry.entry.metadata();
            let name = intern_name(metadata.name())?;
            validate_tier(&name, metadata.tier(), pending_entry.attempted_tier)?;
            let hash = metadata.content_hash();

            if let Some(existing) = staged.get(&name) {
                if existing.tier != pending_entry.attempted_tier {
                    return Err(RegistryError::TierMismatch {
                        name,
                        declared: existing.tier,
                        attempted: pending_entry.attempted_tier,
                    });
                }
                if existing.hash != hash {
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
    Ok(())
}

fn procedure_metadata(pending: &PendingEntry) -> Result<ProcedureMetadata, RegistryError> {
    let metadata = pending.entry.metadata();
    Ok(ProcedureMetadata {
        handle: pending.handle,
        signature: ProcedureSignature {
            parameters: metadata
                .signature_static()
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>, _>>()?,
        },
        output_schema: ProcedureOutputSchema {
            columns: metadata
                .output_columns_static()
                .iter()
                .map(output_column)
                .collect::<Result<Vec<_>, _>>()?,
        },
        tier: pending.attempted_tier,
        mutability: metadata.mutability(),
        capability_required: None,
    })
}

fn parameter(parameter: &StaticParameter) -> Result<ProcedureParameter, RegistryError> {
    Ok(ProcedureParameter {
        name: intern_static(parameter.name, "parameter")?,
        ty: parameter.ty.clone(),
        nullable: parameter.nullable,
    })
}

fn output_column(column: &StaticOutputColumn) -> Result<ProcedureOutputColumn, RegistryError> {
    Ok(ProcedureOutputColumn {
        name: intern_static(column.name, "output column")?,
        ty: column.ty.clone(),
    })
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
