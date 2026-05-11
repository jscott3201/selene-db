//! Drift-protection coverage tables for the pack snapshot corpus.

#![allow(missing_docs)]

/// Every `selene_pack::Gate` variant the pack corpus must exercise at least once.
pub const GATE_COVERAGE: &[PackGate] = PackGate::ALL;

/// Every pack lifecycle event kind the corpus must exercise at least once.
pub const LIFECYCLE_EVENT_COVERAGE: &[PackLifecycleEventKind] = PackLifecycleEventKind::ALL;

/// Stable mirror of `selene_pack::Gate` variants in declaration order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PackGate {
    ManifestSyntaxAndSchema,
    ManifestTypedShape,
    ManifestSchemaVersionSupported,
    PackVersionWellFormed,
    PackNameLexical,
    PackProcedureCountBounded,
    ProcedureNamesUnique,
    ProcedureNameLexical,
    ProcedureWithinPack,
    ReservedNamespace,
    PersistTierRejected,
    TierMutabilityConsistency,
    InlineSchemaSizeBounded,
    InlineSchemaMetaValid,
    PathSchemaSafety,
    ProcedureInputSchemaCompiles,
    ProcedureOutputSchemaCompiles,
    ProcedureCapabilityFormat,
    ProcedureNameLengthBounded,
    ContentHashCanonical,
    ContentHashConsistency,
    ActivationLifecycleAtomicity,
    RegistryConflictDetection,
}

impl PackGate {
    pub const ALL: &'static [Self] = &[
        Self::ManifestSyntaxAndSchema,
        Self::ManifestTypedShape,
        Self::ManifestSchemaVersionSupported,
        Self::PackVersionWellFormed,
        Self::PackNameLexical,
        Self::PackProcedureCountBounded,
        Self::ProcedureNamesUnique,
        Self::ProcedureNameLexical,
        Self::ProcedureWithinPack,
        Self::ReservedNamespace,
        Self::PersistTierRejected,
        Self::TierMutabilityConsistency,
        Self::InlineSchemaSizeBounded,
        Self::InlineSchemaMetaValid,
        Self::PathSchemaSafety,
        Self::ProcedureInputSchemaCompiles,
        Self::ProcedureOutputSchemaCompiles,
        Self::ProcedureCapabilityFormat,
        Self::ProcedureNameLengthBounded,
        Self::ContentHashCanonical,
        Self::ContentHashConsistency,
        Self::ActivationLifecycleAtomicity,
        Self::RegistryConflictDetection,
    ];

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::ManifestSyntaxAndSchema => "manifest_syntax_and_schema",
            Self::ManifestTypedShape => "manifest_typed_shape",
            Self::ManifestSchemaVersionSupported => "manifest_schema_version_supported",
            Self::PackVersionWellFormed => "pack_version_well_formed",
            Self::PackNameLexical => "pack_name_lexical",
            Self::PackProcedureCountBounded => "pack_procedure_count_bounded",
            Self::ProcedureNamesUnique => "procedure_names_unique",
            Self::ProcedureNameLexical => "procedure_name_lexical",
            Self::ProcedureWithinPack => "procedure_within_pack",
            Self::ReservedNamespace => "reserved_namespace",
            Self::PersistTierRejected => "persist_tier_rejected",
            Self::TierMutabilityConsistency => "tier_mutability_consistency",
            Self::InlineSchemaSizeBounded => "inline_schema_size_bounded",
            Self::InlineSchemaMetaValid => "inline_schema_meta_valid",
            Self::PathSchemaSafety => "path_schema_safety",
            Self::ProcedureInputSchemaCompiles => "procedure_input_schema_compiles",
            Self::ProcedureOutputSchemaCompiles => "procedure_output_schema_compiles",
            Self::ProcedureCapabilityFormat => "procedure_capability_format",
            Self::ProcedureNameLengthBounded => "procedure_name_length_bounded",
            Self::ContentHashCanonical => "content_hash_canonical",
            Self::ContentHashConsistency => "content_hash_consistency",
            Self::ActivationLifecycleAtomicity => "activation_lifecycle_atomicity",
            Self::RegistryConflictDetection => "registry_conflict_detection",
        }
    }
}

/// Stable mirror of pack lifecycle event variants.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PackLifecycleEventKind {
    ValidationFailed,
    Staged,
    Activated,
    Deprecated,
    Disabled,
}

impl PackLifecycleEventKind {
    pub const ALL: &'static [Self] = &[
        Self::ValidationFailed,
        Self::Staged,
        Self::Activated,
        Self::Deprecated,
        Self::Disabled,
    ];
}
