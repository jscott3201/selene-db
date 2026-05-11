//! Observed coverage helpers for the pack snapshot corpus.

use selene_pack::{Gate, LifecycleEvent};
use selene_testing::{
    PackGate, PackHistoryWalEntry, PackLifecycleEventKind, PackLifecycleEventPayload,
};

pub(crate) const MANIFEST_PATH_GATES: &[PackGate] = &[
    PackGate::ManifestSyntaxAndSchema,
    PackGate::ManifestTypedShape,
    PackGate::ManifestSchemaVersionSupported,
    PackGate::PackVersionWellFormed,
    PackGate::PackNameLexical,
    PackGate::PackProcedureCountBounded,
    PackGate::ProcedureNamesUnique,
    PackGate::ProcedureNameLexical,
    PackGate::ReservedNamespace,
    PackGate::ProcedureWithinPack,
    PackGate::PersistTierRejected,
    PackGate::TierMutabilityConsistency,
    PackGate::ProcedureNameLengthBounded,
    PackGate::InlineSchemaSizeBounded,
    PackGate::InlineSchemaMetaValid,
    PackGate::PathSchemaSafety,
    PackGate::ProcedureInputSchemaCompiles,
    PackGate::ProcedureOutputSchemaCompiles,
    PackGate::ProcedureCapabilityFormat,
    PackGate::ContentHashCanonical,
    PackGate::ContentHashConsistency,
];

pub(crate) const LIFECYCLE_PATH_GATES_STAGE: &[PackGate] =
    &[PackGate::ActivationLifecycleAtomicity];

pub(crate) const LIFECYCLE_PATH_GATES_ACTIVATE: &[PackGate] =
    &[PackGate::RegistryConflictDetection];

pub(crate) fn gate_to_pack(gate: Gate) -> PackGate {
    match gate {
        Gate::ManifestSyntaxAndSchema => PackGate::ManifestSyntaxAndSchema,
        Gate::ManifestTypedShape => PackGate::ManifestTypedShape,
        Gate::ManifestSchemaVersionSupported => PackGate::ManifestSchemaVersionSupported,
        Gate::PackVersionWellFormed => PackGate::PackVersionWellFormed,
        Gate::PackNameLexical => PackGate::PackNameLexical,
        Gate::PackProcedureCountBounded => PackGate::PackProcedureCountBounded,
        Gate::ProcedureNamesUnique => PackGate::ProcedureNamesUnique,
        Gate::ProcedureNameLexical => PackGate::ProcedureNameLexical,
        Gate::ProcedureWithinPack => PackGate::ProcedureWithinPack,
        Gate::ReservedNamespace => PackGate::ReservedNamespace,
        Gate::PersistTierRejected => PackGate::PersistTierRejected,
        Gate::TierMutabilityConsistency => PackGate::TierMutabilityConsistency,
        Gate::InlineSchemaSizeBounded => PackGate::InlineSchemaSizeBounded,
        Gate::InlineSchemaMetaValid => PackGate::InlineSchemaMetaValid,
        Gate::PathSchemaSafety => PackGate::PathSchemaSafety,
        Gate::ProcedureInputSchemaCompiles => PackGate::ProcedureInputSchemaCompiles,
        Gate::ProcedureOutputSchemaCompiles => PackGate::ProcedureOutputSchemaCompiles,
        Gate::ProcedureCapabilityFormat => PackGate::ProcedureCapabilityFormat,
        Gate::ProcedureNameLengthBounded => PackGate::ProcedureNameLengthBounded,
        Gate::ContentHashCanonical => PackGate::ContentHashCanonical,
        Gate::ContentHashConsistency => PackGate::ContentHashConsistency,
        Gate::ActivationLifecycleAtomicity => PackGate::ActivationLifecycleAtomicity,
        Gate::RegistryConflictDetection => PackGate::RegistryConflictDetection,
        _ => panic!("unknown selene_pack::Gate variant"),
    }
}

pub(crate) fn lifecycle_event_kind(event: &LifecycleEvent) -> PackLifecycleEventKind {
    match event {
        LifecycleEvent::ValidationFailed { .. } => PackLifecycleEventKind::ValidationFailed,
        LifecycleEvent::Staged { .. } => PackLifecycleEventKind::Staged,
        LifecycleEvent::Activated { .. } => PackLifecycleEventKind::Activated,
        LifecycleEvent::Deprecated { .. } => PackLifecycleEventKind::Deprecated,
        LifecycleEvent::Disabled { .. } => PackLifecycleEventKind::Disabled,
        _ => panic!("unknown selene_pack::LifecycleEvent variant"),
    }
}

pub(crate) fn history_event_kind(entry: &PackHistoryWalEntry) -> Option<PackLifecycleEventKind> {
    match entry {
        PackHistoryWalEntry::Lifecycle(payload) => Some(lifecycle_payload_kind(payload)),
        PackHistoryWalEntry::LegacyProcedurePackActivated { .. }
        | PackHistoryWalEntry::LegacyProcedurePackDeprecated { .. }
        | PackHistoryWalEntry::LegacyProcedurePackDisabled { .. }
        | PackHistoryWalEntry::NodeCreated { .. } => None,
        _ => unreachable!("unknown pack history WAL entry"),
    }
}

fn lifecycle_payload_kind(payload: &PackLifecycleEventPayload) -> PackLifecycleEventKind {
    match payload {
        PackLifecycleEventPayload::ValidationFailed { .. } => {
            PackLifecycleEventKind::ValidationFailed
        }
        PackLifecycleEventPayload::Staged { .. } => PackLifecycleEventKind::Staged,
        PackLifecycleEventPayload::Activated { .. } => PackLifecycleEventKind::Activated,
        PackLifecycleEventPayload::Deprecated { .. } => PackLifecycleEventKind::Deprecated,
        PackLifecycleEventPayload::Disabled { .. } => PackLifecycleEventKind::Disabled,
        _ => unreachable!("unknown pack lifecycle payload"),
    }
}
