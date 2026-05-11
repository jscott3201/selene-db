//! Curated procedure-pack snapshot corpus fixtures.

#![allow(missing_docs)]

pub mod coverage;
pub mod fixtures;

pub use coverage::{GATE_COVERAGE, LIFECYCLE_EVENT_COVERAGE, PackGate, PackLifecycleEventKind};
pub use fixtures::{
    PackCorpusFixture, PackHistoryWalEntry, PackLifecycleEventPayload, PackLifecycleSinkMode,
    PackLifecycleStep, PackManifestFixture, SyntheticManifestSpec, SyntheticMutability,
    SyntheticProcedureSpec, SyntheticTier,
};

#[derive(Clone, Debug)]
pub struct PackCorpus {
    entries: Vec<PackCorpusEntry>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PackCorpusEntry {
    pub slug: &'static str,
    pub description: &'static str,
    pub fixture: PackCorpusFixture,
    pub category: PackCorpusCategory,
    pub covered_gates: &'static [PackGate],
    pub covered_events: &'static [PackLifecycleEventKind],
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PackCorpusCategory {
    Manifest,
    Lifecycle,
    Hash,
    History,
}

impl PackCorpus {
    #[must_use]
    pub fn m5e() -> Self {
        Self {
            entries: ENTRIES.to_vec(),
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &PackCorpusEntry> {
        self.entries.iter()
    }
}

use PackCorpusCategory::*;
use PackCorpusFixture::*;
use PackGate::*;
use PackLifecycleEventKind::*;
use PackLifecycleSinkMode::*;
use PackLifecycleStep::*;
use PackManifestFixture::*;
use SyntheticMutability::*;
use SyntheticTier::*;

const OBJ: &str = r#"{"type":"object"}"#;
const ARR: &str = r#"{"type":"array","items":{"type":"string"}}"#;

const VALID_PROC: SyntheticProcedureSpec = SyntheticProcedureSpec {
    name_suffix: "echo",
    tier: Graph,
    mutability: Read,
    input_schema_inline: OBJ,
    output_schema_inline: OBJ,
    capability: None,
};

const VALID_PROC_CAP: SyntheticProcedureSpec = SyntheticProcedureSpec {
    name_suffix: "cap_echo",
    tier: Graph,
    mutability: Read,
    input_schema_inline: OBJ,
    output_schema_inline: OBJ,
    capability: Some("pack:read"),
};

const MUTATION_PROC: SyntheticProcedureSpec = SyntheticProcedureSpec {
    name_suffix: "write",
    tier: Mutation,
    mutability: Write,
    input_schema_inline: OBJ,
    output_schema_inline: OBJ,
    capability: Some("pack:write"),
};

const MINIMAL_PROCS: &[SyntheticProcedureSpec] = &[VALID_PROC];
const MULTI_PROCS: &[SyntheticProcedureSpec] = &[VALID_PROC, VALID_PROC_CAP, MUTATION_PROC];
const ARRAY_PROCS: &[SyntheticProcedureSpec] = &[SyntheticProcedureSpec {
    name_suffix: "list",
    tier: Graph,
    mutability: Read,
    input_schema_inline: ARR,
    output_schema_inline: ARR,
    capability: None,
}];

const MINIMAL_MANIFEST: SyntheticManifestSpec = SyntheticManifestSpec {
    pack_name: "snap",
    pack_version: "1.0.0",
    procedures: MINIMAL_PROCS,
};
const MULTI_MANIFEST: SyntheticManifestSpec = SyntheticManifestSpec {
    pack_name: "multi",
    pack_version: "2.0.0",
    procedures: MULTI_PROCS,
};
const ARRAY_MANIFEST: SyntheticManifestSpec = SyntheticManifestSpec {
    pack_name: "arrays",
    pack_version: "3.1.4",
    procedures: ARRAY_PROCS,
};

const BAD_JSON: &[u8] = b"{";
const BAD_SCHEMA_VERSION: &[u8] = br#"{
  "schema_version": 99,
  "pack_name": "badversion",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": []
}"#;
const BAD_PACK_NAME: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "1bad",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": []
}"#;
const BAD_PROC_NAME: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": [{
    "name": "demo",
    "tier": "graph",
    "mutability": "read",
    "input_schema": { "inline": { "type": "object" } },
    "output_schema": { "inline": { "type": "object" } },
    "capability_required": null
  }]
}"#;
const RESERVED_PROC: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": [{
    "name": "selene.bad",
    "tier": "graph",
    "mutability": "read",
    "input_schema": { "inline": { "type": "object" } },
    "output_schema": { "inline": { "type": "object" } },
    "capability_required": null
  }]
}"#;
const PERSIST_PROC: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": [{
    "name": "demo.persist",
    "tier": "persist",
    "mutability": "admin",
    "input_schema": { "inline": { "type": "object" } },
    "output_schema": { "inline": { "type": "object" } },
    "capability_required": null
  }]
}"#;
const TIER_MISMATCH: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": [{
    "name": "demo.write",
    "tier": "graph",
    "mutability": "graph_write",
    "input_schema": { "inline": { "type": "object" } },
    "output_schema": { "inline": { "type": "object" } },
    "capability_required": null
  }]
}"#;
const BAD_CAPABILITY: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": [{
    "name": "demo.cap",
    "tier": "graph",
    "mutability": "read",
    "input_schema": { "inline": { "type": "object" } },
    "output_schema": { "inline": { "type": "object" } },
    "capability_required": "bad cap!"
  }]
}"#;
const BAD_HASH_FORMAT: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
  "procedures": []
}"#;
const BAD_HASH_MISMATCH: &[u8] = br#"{
  "schema_version": 1,
  "pack_name": "demo",
  "pack_version": "1.0.0",
  "content_hash": "blake3:1111111111111111111111111111111111111111111111111111111111111111",
  "procedures": []
}"#;

const FULL_SCRIPT: &[PackLifecycleStep] = &[
    Upload {
        raw_bytes: b"__synthetic:minimal__",
        principal: "snapshot.principal",
    },
    Validate,
    Stage,
    Activate,
    Deprecate {
        reason: "snapshot.reason",
    },
    Disable,
];
const VALIDATION_FAILED_SCRIPT: &[PackLifecycleStep] = &[
    Upload {
        raw_bytes: BAD_JSON,
        principal: "snapshot.bad",
    },
    Validate,
    AssertValidationFailed,
];
const SINK_REFUSED_SCRIPT: &[PackLifecycleStep] = &[
    Upload {
        raw_bytes: b"__synthetic:minimal__",
        principal: "snapshot.refused",
    },
    Validate,
    Stage,
];
const CONFLICT_SCRIPT: &[PackLifecycleStep] = &[
    Upload {
        raw_bytes: b"__synthetic:minimal__",
        principal: "snapshot.first",
    },
    Validate,
    Stage,
    Activate,
    Upload {
        raw_bytes: b"__synthetic:minimal__",
        principal: "snapshot.second",
    },
    Validate,
    Stage,
    Activate,
];

const FULL_EVENT_KINDS: &[PackLifecycleEventKind] = &[Staged, Activated, Deprecated, Disabled];
const VALIDATION_FAILED_KIND: &[PackLifecycleEventKind] = &[ValidationFailed];
const STAGED_KIND: &[PackLifecycleEventKind] = &[Staged];
const STAGED_ACTIVATED_KIND: &[PackLifecycleEventKind] = &[Staged, Activated];
const HISTORY_FULL_KINDS: &[PackLifecycleEventKind] =
    &[ValidationFailed, Staged, Activated, Deprecated, Disabled];

macro_rules! entry {
    ($slug:literal, $desc:literal, $fixture:expr, $category:expr, [$($gate:ident),* $(,)?], [$($event:ident),* $(,)?]) => {
        PackCorpusEntry {
            slug: $slug,
            description: $desc,
            fixture: $fixture,
            category: $category,
            covered_gates: &[$(PackGate::$gate),*],
            covered_events: &[$(PackLifecycleEventKind::$event),*],
        }
    };
}

const ENTRIES: &[PackCorpusEntry] = &[
    PackCorpusEntry {
        slug: "manifest_valid_minimal",
        description: "valid one-procedure manifest covers full gate taxonomy",
        fixture: ManifestParse {
            manifest: Synthetic(&MINIMAL_MANIFEST),
        },
        category: Manifest,
        covered_gates: GATE_COVERAGE,
        covered_events: &[],
    },
    entry!(
        "manifest_invalid_json",
        "syntax error maps to manifest_syntax_and_schema",
        ManifestParse {
            manifest: Static(BAD_JSON)
        },
        Manifest,
        [ManifestSyntaxAndSchema],
        []
    ),
    entry!(
        "manifest_bad_schema_version",
        "unsupported schema_version is rejected",
        ManifestParse {
            manifest: Static(BAD_SCHEMA_VERSION)
        },
        Manifest,
        [ManifestSchemaVersionSupported],
        []
    ),
    entry!(
        "manifest_bad_pack_name",
        "invalid pack_name stable-field rendering",
        ManifestParse {
            manifest: Static(BAD_PACK_NAME)
        },
        Manifest,
        [PackNameLexical],
        []
    ),
    entry!(
        "manifest_bad_proc_name",
        "invalid procedure name stable-field rendering",
        ManifestParse {
            manifest: Static(BAD_PROC_NAME)
        },
        Manifest,
        [ProcedureNameLexical],
        []
    ),
    entry!(
        "manifest_reserved_namespace",
        "reserved selene namespace is rejected",
        ManifestParse {
            manifest: Static(RESERVED_PROC)
        },
        Manifest,
        [ReservedNamespace],
        []
    ),
    entry!(
        "manifest_persist_tier",
        "persist-tier declarations are rejected in v1",
        ManifestParse {
            manifest: Static(PERSIST_PROC)
        },
        Manifest,
        [PersistTierRejected],
        []
    ),
    entry!(
        "manifest_tier_mismatch",
        "tier and mutability drift is rejected",
        ManifestParse {
            manifest: Static(TIER_MISMATCH)
        },
        Manifest,
        [TierMutabilityConsistency],
        []
    ),
    entry!(
        "manifest_bad_capability",
        "malformed capability is rejected",
        ManifestParse {
            manifest: Static(BAD_CAPABILITY)
        },
        Manifest,
        [ProcedureCapabilityFormat],
        []
    ),
    entry!(
        "manifest_bad_hash_format",
        "non-blake3 content_hash is rejected",
        ManifestParse {
            manifest: Static(BAD_HASH_FORMAT)
        },
        Manifest,
        [ContentHashCanonical],
        []
    ),
    entry!(
        "manifest_hash_mismatch",
        "declared content_hash must match canonical bytes",
        ManifestParse {
            manifest: Static(BAD_HASH_MISMATCH)
        },
        Manifest,
        [ContentHashConsistency],
        []
    ),
    PackCorpusEntry {
        slug: "lifecycle_full",
        description: "complete lifecycle through disabled terminal state",
        fixture: LifecycleRun {
            script: FULL_SCRIPT,
            sink_mode: Capture,
        },
        category: Lifecycle,
        covered_gates: &[RegistryConflictDetection, ActivationLifecycleAtomicity],
        covered_events: FULL_EVENT_KINDS,
    },
    PackCorpusEntry {
        slug: "lifecycle_validation_failed",
        description: "manifest validation failure is captured as lifecycle event",
        fixture: LifecycleRun {
            script: VALIDATION_FAILED_SCRIPT,
            sink_mode: Capture,
        },
        category: Lifecycle,
        covered_gates: &[ManifestSyntaxAndSchema],
        covered_events: VALIDATION_FAILED_KIND,
    },
    PackCorpusEntry {
        slug: "lifecycle_sink_refused",
        description: "sink refusal is surfaced without registry mutation",
        fixture: LifecycleRun {
            script: SINK_REFUSED_SCRIPT,
            sink_mode: RefuseAtStep {
                step_index: 1,
                detail: "test refuses staging",
            },
        },
        category: Lifecycle,
        covered_gates: &[],
        covered_events: &[],
    },
    PackCorpusEntry {
        slug: "lifecycle_activation_conflict",
        description: "active registry occupancy rejects second activation",
        fixture: LifecycleRun {
            script: CONFLICT_SCRIPT,
            sink_mode: Capture,
        },
        category: Lifecycle,
        covered_gates: &[RegistryConflictDetection],
        covered_events: STAGED_ACTIVATED_KIND,
    },
    entry!(
        "hash_minimal",
        "canonical bytes and hash for minimal manifest",
        HashCanonical {
            manifest: Synthetic(&MINIMAL_MANIFEST)
        },
        Hash,
        [ContentHashCanonical, ContentHashConsistency],
        []
    ),
    entry!(
        "hash_multi_procedure",
        "canonical bytes and hash for multi-procedure manifest",
        HashCanonical {
            manifest: Synthetic(&MULTI_MANIFEST)
        },
        Hash,
        [ContentHashCanonical, ContentHashConsistency],
        []
    ),
    entry!(
        "hash_schema_shape",
        "canonical bytes and hash for array schema manifest",
        HashCanonical {
            manifest: Synthetic(&ARRAY_MANIFEST)
        },
        Hash,
        [ContentHashCanonical, ContentHashConsistency],
        []
    ),
    entry!(
        "history_empty_wal",
        "empty WAL produces no history rows",
        HistoryReplay { entries: &[] },
        History,
        [ActivationLifecycleAtomicity],
        []
    ),
    entry!(
        "history_single_staged",
        "single staged WAL lifecycle row",
        HistoryReplay {
            entries: &[PackHistoryWalEntry::Lifecycle(
                PackLifecycleEventPayload::Staged {
                    pack_name: "history_pack",
                    content_hash: [1_u8; 32],
                    principal: "history.principal",
                    at_seconds: 1,
                },
            )],
        },
        History,
        [ActivationLifecycleAtomicity],
        [Staged]
    ),
    PackCorpusEntry {
        slug: "history_full_lifecycle",
        description: "all lifecycle variants project to history rows",
        fixture: HistoryReplay {
            entries: &[
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::ValidationFailed {
                    pack_name: Some("bad_pack"),
                    principal: "history.principal",
                    error: "manifest failed",
                    at_seconds: 1,
                }),
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::Staged {
                    pack_name: "full_pack",
                    content_hash: [2_u8; 32],
                    principal: "history.principal",
                    at_seconds: 2,
                }),
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::Activated {
                    pack_name: "full_pack",
                    content_hash: [2_u8; 32],
                    principal: "history.principal",
                    at_seconds: 3,
                }),
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::Deprecated {
                    pack_name: "full_pack",
                    reason: "old",
                    principal: "history.principal",
                    at_seconds: 4,
                }),
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::Disabled {
                    pack_name: "full_pack",
                    principal: "history.principal",
                    at_seconds: 5,
                }),
            ],
        },
        category: History,
        covered_gates: &[ActivationLifecycleAtomicity],
        covered_events: HISTORY_FULL_KINDS,
    },
    PackCorpusEntry {
        slug: "history_skips_all_legacy_variants",
        description: "legacy procedure-pack schema changes are skipped",
        fixture: HistoryReplay {
            entries: &[
                PackHistoryWalEntry::LegacyProcedurePackActivated {
                    pack_name: "legacy_pack",
                    version: "0.1.0",
                },
                PackHistoryWalEntry::LegacyProcedurePackDeprecated {
                    pack_name: "legacy_pack",
                    version: "0.1.0",
                    reason: "legacy",
                },
                PackHistoryWalEntry::LegacyProcedurePackDisabled {
                    pack_name: "legacy_pack",
                    version: "0.1.0",
                    reason: "legacy",
                },
                PackHistoryWalEntry::NodeCreated {
                    id: 1,
                    label: "HistoryNode",
                },
                PackHistoryWalEntry::Lifecycle(PackLifecycleEventPayload::Staged {
                    pack_name: "real_pack",
                    content_hash: [3_u8; 32],
                    principal: "history.principal",
                    at_seconds: 9,
                }),
            ],
        },
        category: History,
        covered_gates: &[ActivationLifecycleAtomicity],
        covered_events: STAGED_KIND,
    },
];
