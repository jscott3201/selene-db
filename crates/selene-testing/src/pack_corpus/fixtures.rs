//! Static fixture inputs for the pack snapshot corpus.

#![allow(missing_docs)]

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PackCorpusFixture {
    ManifestParse {
        manifest: PackManifestFixture,
    },
    LifecycleRun {
        script: &'static [PackLifecycleStep],
        sink_mode: PackLifecycleSinkMode,
    },
    HashCanonical {
        manifest: PackManifestFixture,
    },
    HistoryReplay {
        entries: &'static [PackHistoryWalEntry],
    },
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum PackManifestFixture {
    Static(&'static [u8]),
    Synthetic(&'static SyntheticManifestSpec),
}

#[derive(Clone, Copy, Debug)]
pub struct SyntheticManifestSpec {
    pub pack_name: &'static str,
    pub pack_version: &'static str,
    pub procedures: &'static [SyntheticProcedureSpec],
}

#[derive(Clone, Copy, Debug)]
pub struct SyntheticProcedureSpec {
    pub name_suffix: &'static str,
    pub tier: SyntheticTier,
    pub mutability: SyntheticMutability,
    pub input_schema_inline: &'static str,
    pub output_schema_inline: &'static str,
    pub capability: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub enum SyntheticTier {
    Graph,
    Mutation,
}

#[derive(Clone, Copy, Debug)]
pub enum SyntheticMutability {
    Read,
    Write,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PackLifecycleStep {
    Upload {
        raw_bytes: &'static [u8],
        principal: &'static str,
    },
    Validate,
    Stage,
    Activate,
    Deprecate {
        reason: &'static str,
    },
    Disable,
    AssertValidationFailed,
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum PackLifecycleSinkMode {
    Capture,
    RefuseAtStep {
        step_index: usize,
        detail: &'static str,
    },
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PackHistoryWalEntry {
    Lifecycle(PackLifecycleEventPayload),
    LegacyProcedurePackActivated {
        pack_name: &'static str,
        version: &'static str,
    },
    LegacyProcedurePackDeprecated {
        pack_name: &'static str,
        version: &'static str,
        reason: &'static str,
    },
    LegacyProcedurePackDisabled {
        pack_name: &'static str,
        version: &'static str,
        reason: &'static str,
    },
    NodeCreated {
        id: u64,
        label: &'static str,
    },
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PackLifecycleEventPayload {
    ValidationFailed {
        pack_name: Option<&'static str>,
        principal: &'static str,
        error: &'static str,
        at_seconds: i64,
    },
    Staged {
        pack_name: &'static str,
        content_hash: [u8; 32],
        principal: &'static str,
        at_seconds: i64,
    },
    Activated {
        pack_name: &'static str,
        content_hash: [u8; 32],
        principal: &'static str,
        at_seconds: i64,
    },
    Deprecated {
        pack_name: &'static str,
        reason: &'static str,
        principal: &'static str,
        at_seconds: i64,
    },
    Disabled {
        pack_name: &'static str,
        principal: &'static str,
        at_seconds: i64,
    },
}
