//! Validation gate taxonomy and coverage slices for procedure-pack manifests.
//!
//! Gate-to-error ownership map:
//!
//! | Gate | Owns ManifestError variant(s) |
//! |---|---|
//! | `ManifestSyntaxAndSchema` | `InvalidJson`, `SchemaViolation` |
//! | `ManifestTypedShape` | `DeserializeError` |
//! | `ManifestSchemaVersionSupported` | `UnsupportedSchemaVersion` |
//! | `ContentHashPlaceholderRecognized` | `UnsupportedContentHash` |
//! | `PackVersionWellFormed` | `InvalidPackVersion` |
//! | `PackNameLexical` | `InvalidPackName` |
//! | `PackProcedureCountBounded` | `PackProcedureCountExceedsBound` |
//! | `ProcedureNamesUnique` | `DuplicateProcedureName` |
//! | `ProcedureNameLexical` | `InvalidProcedureName` |
//! | `ProcedureWithinPack` | `ProcedureNameOutsidePack` |
//! | `ReservedNamespace` | `ReservedNamespaceConflict` |
//! | `PersistTierRejected` | `PersistTierInManifest` |
//! | `TierMutabilityConsistency` | `MutabilityTierMismatch` |
//! | `InlineSchemaSizeBounded` | `InlineSchemaSizeExceedsBound` |
//! | `InlineSchemaMetaValid` | `InvalidInlineSchema` |
//! | `PathSchemaSafety` | `InvalidSchemaPath` |
//! | `ProcedureInputSchemaCompiles` | `ProcedureSchemaCompileFailed { field: "input_schema" }` |
//! | `ProcedureOutputSchemaCompiles` | `ProcedureSchemaCompileFailed { field: "output_schema" }` |
//! | `ProcedureCapabilityFormat` | `ProcedureCapabilityMalformed` |
//! | `ProcedureNameLengthBounded` | `ProcedureNameTooLong` |
//! | `ContentHashCanonical` | BRIEF-48 deferred |
//! | `ContentHashConsistency` | BRIEF-48 deferred |
//! | `ActivationLifecycleAtomicity` | BRIEF-46 deferred |
//! | `RegistryConflictDetection` | activation seal conflict detection |

/// Maximum procedure entries accepted in one procedure-pack manifest.
pub const MAX_PROCEDURES_PER_PACK: usize = 256;

/// Maximum serialized size for an inline JSON Schema document.
pub const MAX_INLINE_SCHEMA_SIZE_BYTES: usize = 64 * 1024;

/// Maximum full procedure-name byte length.
pub const MAX_PROCEDURE_NAME_LENGTH: usize = 255;

/// Non-exhaustive public validation gate taxonomy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Gate {
    /// JSON syntax and generated manifest JSON Schema validation.
    ManifestSyntaxAndSchema,
    /// Typed manifest deserialization.
    ManifestTypedShape,
    /// Manifest `schema_version` is supported by this binary.
    ManifestSchemaVersionSupported,
    /// Manifest `content_hash` is the BRIEF-43 placeholder.
    ContentHashPlaceholderRecognized,
    /// Manifest `pack_version` is valid semver.
    PackVersionWellFormed,
    /// Manifest `pack_name` follows canonical lexical rules.
    PackNameLexical,
    /// Manifest procedure count is within the v1.0 bound.
    PackProcedureCountBounded,
    /// Procedure names are unique within the manifest.
    ProcedureNamesUnique,
    /// Procedure name follows canonical lexical rules.
    ProcedureNameLexical,
    /// Procedure name is within the declaring pack prefix.
    ProcedureWithinPack,
    /// Procedure name does not claim the platform-reserved namespace.
    ReservedNamespace,
    /// Persist-tier procedures are rejected in v1.0 manifests.
    PersistTierRejected,
    /// Declared tier matches declared mutability.
    TierMutabilityConsistency,
    /// Inline schema serialized size is within the v1.0 bound.
    InlineSchemaSizeBounded,
    /// Inline schema is valid JSON Schema 2020-12.
    InlineSchemaMetaValid,
    /// Path schema reference is a safe relative path.
    PathSchemaSafety,
    /// Inline input schema compiles as JSON Schema 2020-12.
    ProcedureInputSchemaCompiles,
    /// Inline output schema compiles as JSON Schema 2020-12.
    ProcedureOutputSchemaCompiles,
    /// Optional capability declaration has canonical shape.
    ProcedureCapabilityFormat,
    /// Procedure name byte length is within the v1.0 bound.
    ProcedureNameLengthBounded,
    /// Canonical content hash enforcement, deferred to BRIEF-48.
    ContentHashCanonical,
    /// Content hash consistency enforcement, deferred to BRIEF-48.
    ContentHashConsistency,
    /// Activation lifecycle atomicity enforcement, deferred to BRIEF-46.
    ActivationLifecycleAtomicity,
    /// Registry conflict detection enforcement at activation seal time.
    RegistryConflictDetection,
}

impl Gate {
    /// Every known gate variant.
    pub const ALL: &'static [Self] = &[
        Self::ManifestSyntaxAndSchema,
        Self::ManifestTypedShape,
        Self::ManifestSchemaVersionSupported,
        Self::ContentHashPlaceholderRecognized,
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

    /// Stable snake_case gate identifier for telemetry and logs.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::ManifestSyntaxAndSchema => "manifest_syntax_and_schema",
            Self::ManifestTypedShape => "manifest_typed_shape",
            Self::ManifestSchemaVersionSupported => "manifest_schema_version_supported",
            Self::ContentHashPlaceholderRecognized => "content_hash_placeholder_recognized",
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

    /// One-line human-readable gate description.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ManifestSyntaxAndSchema => {
                "manifest JSON parses and matches the generated JSON Schema"
            }
            Self::ManifestTypedShape => "manifest JSON deserializes into the typed shape",
            Self::ManifestSchemaVersionSupported => "manifest schema_version is supported",
            Self::ContentHashPlaceholderRecognized => {
                "manifest content_hash uses the v1.0 placeholder"
            }
            Self::PackVersionWellFormed => "manifest pack_version is valid semver",
            Self::PackNameLexical => "manifest pack_name is a canonical ASCII segment",
            Self::PackProcedureCountBounded => "manifest procedure count is within bounds",
            Self::ProcedureNamesUnique => "manifest procedure names are unique",
            Self::ProcedureNameLexical => "procedure name is a canonical dot-joined ASCII path",
            Self::ProcedureWithinPack => "procedure name is inside the declaring pack prefix",
            Self::ReservedNamespace => "procedure name avoids the reserved selene namespace",
            Self::PersistTierRejected => "persist-tier procedures are rejected in v1.0",
            Self::TierMutabilityConsistency => "procedure tier matches declared mutability",
            Self::InlineSchemaSizeBounded => "inline schema serialized size is within bounds",
            Self::InlineSchemaMetaValid => "inline schema is valid JSON Schema 2020-12",
            Self::PathSchemaSafety => "path schema reference is a safe relative path",
            Self::ProcedureInputSchemaCompiles => "inline input schema compiles",
            Self::ProcedureOutputSchemaCompiles => "inline output schema compiles",
            Self::ProcedureCapabilityFormat => "capability declaration has canonical shape",
            Self::ProcedureNameLengthBounded => "procedure name byte length is within bounds",
            Self::ContentHashCanonical => "canonical content hash is enforced",
            Self::ContentHashConsistency => "content hash matches manifest payload",
            Self::ActivationLifecycleAtomicity => "activation lifecycle is atomic",
            Self::RegistryConflictDetection => "activation registry conflicts are detected",
        }
    }
}

/// Manifest-level gates, evaluated once after typed deserialization.
pub const MANIFEST_LEVEL_GATES: &[Gate] = &[
    Gate::ManifestSyntaxAndSchema,
    Gate::ManifestTypedShape,
    Gate::ManifestSchemaVersionSupported,
    Gate::ContentHashPlaceholderRecognized,
    Gate::PackVersionWellFormed,
    Gate::PackNameLexical,
    Gate::PackProcedureCountBounded,
    Gate::ProcedureNamesUnique,
];

/// Procedure-level gates, evaluated for each procedure in source order.
pub const PROCEDURE_LEVEL_GATES: &[Gate] = &[
    Gate::ProcedureNameLexical,
    Gate::ReservedNamespace,
    Gate::ProcedureWithinPack,
    Gate::PersistTierRejected,
    Gate::TierMutabilityConsistency,
    Gate::ProcedureNameLengthBounded,
    Gate::InlineSchemaSizeBounded,
    Gate::InlineSchemaMetaValid,
    Gate::PathSchemaSafety,
    Gate::ProcedureInputSchemaCompiles,
    Gate::ProcedureOutputSchemaCompiles,
    Gate::ProcedureCapabilityFormat,
];

/// All manifest-validation gates in canonical evaluation order.
pub const MANIFEST_VALIDATION_COVERAGE: &[Gate] = &[
    Gate::ManifestSyntaxAndSchema,
    Gate::ManifestTypedShape,
    Gate::ManifestSchemaVersionSupported,
    Gate::ContentHashPlaceholderRecognized,
    Gate::PackVersionWellFormed,
    Gate::PackNameLexical,
    Gate::PackProcedureCountBounded,
    Gate::ProcedureNamesUnique,
    Gate::ProcedureNameLexical,
    Gate::ReservedNamespace,
    Gate::ProcedureWithinPack,
    Gate::PersistTierRejected,
    Gate::TierMutabilityConsistency,
    Gate::ProcedureNameLengthBounded,
    Gate::InlineSchemaSizeBounded,
    Gate::InlineSchemaMetaValid,
    Gate::PathSchemaSafety,
    Gate::ProcedureInputSchemaCompiles,
    Gate::ProcedureOutputSchemaCompiles,
    Gate::ProcedureCapabilityFormat,
];

/// Activation-seal gates enforced in v1.0.
pub const ACTIVATION_SEAL_COVERAGE: &[Gate] = &[Gate::RegistryConflictDetection];

/// Known validation gates deferred to later M5e briefs.
pub const DEFERRED_GATES: &[Gate] = &[
    Gate::ContentHashCanonical,
    Gate::ContentHashConsistency,
    Gate::ActivationLifecycleAtomicity,
];
