//! Manifest fixture materialization for pack snapshots.

use selene_pack::{
    ManifestError, ManifestMutability, ManifestProcedureEntry, ManifestSchemaRef, ManifestTier,
    ProcedurePackManifest, parse_manifest,
};
use selene_testing::{
    PackManifestFixture, SyntheticManifestSpec, SyntheticMutability, SyntheticProcedureSpec,
    SyntheticTier,
};
use serde_json::Value;

use crate::common::manifest_fixture::build_manifest_with_canonical_hash;

const SYNTHETIC_MINIMAL_TOKEN: &[u8] = b"__synthetic:minimal__";
const UPLOAD_PROC: SyntheticProcedureSpec = SyntheticProcedureSpec {
    name_suffix: "echo",
    tier: SyntheticTier::Graph,
    mutability: SyntheticMutability::Read,
    input_schema_inline: r#"{"type":"object"}"#,
    output_schema_inline: r#"{"type":"object"}"#,
    capability: None,
};
const UPLOAD_PROCS: &[SyntheticProcedureSpec] = &[UPLOAD_PROC];
const UPLOAD_SPEC: SyntheticManifestSpec = SyntheticManifestSpec {
    pack_name: "snap",
    pack_version: "1.0.0",
    procedures: UPLOAD_PROCS,
};

pub fn materialize_manifest(
    fixture: &PackManifestFixture,
) -> Result<ProcedurePackManifest, ManifestError> {
    match fixture {
        PackManifestFixture::Static(bytes) => parse_manifest(bytes),
        PackManifestFixture::Synthetic(spec) => {
            let manifest = build_synthetic_manifest(spec);
            let bytes = serde_json::to_vec(&manifest).expect("test manifest serializes");
            parse_manifest(&bytes)
        }
        _ => unreachable!("unknown pack manifest fixture"),
    }
}

pub fn upload_bytes(raw: &[u8]) -> Vec<u8> {
    if raw == SYNTHETIC_MINIMAL_TOKEN {
        let manifest = build_synthetic_manifest(&UPLOAD_SPEC);
        serde_json::to_vec(&manifest).expect("test manifest serializes")
    } else {
        raw.to_vec()
    }
}

fn build_synthetic_manifest(spec: &SyntheticManifestSpec) -> ProcedurePackManifest {
    let procedures = spec
        .procedures
        .iter()
        .map(|procedure| build_synthetic_procedure(spec.pack_name, procedure))
        .collect::<Vec<_>>();
    build_manifest_with_canonical_hash(spec.pack_name, spec.pack_version, procedures)
}

fn build_synthetic_procedure(
    pack_name: &str,
    procedure: &SyntheticProcedureSpec,
) -> ManifestProcedureEntry {
    ManifestProcedureEntry {
        name: format!("{pack_name}.{}", procedure.name_suffix),
        tier: match procedure.tier {
            SyntheticTier::Graph => ManifestTier::Graph,
            SyntheticTier::Mutation => ManifestTier::Mutation,
        },
        mutability: match procedure.mutability {
            SyntheticMutability::Read => ManifestMutability::Read,
            SyntheticMutability::Write => ManifestMutability::GraphWrite,
        },
        input_schema: ManifestSchemaRef::Inline(parse_inline_schema(procedure.input_schema_inline)),
        output_schema: ManifestSchemaRef::Inline(parse_inline_schema(
            procedure.output_schema_inline,
        )),
        capability_required: procedure.capability.map(str::to_owned),
    }
}

fn parse_inline_schema(raw: &str) -> Value {
    serde_json::from_str(raw).expect("synthetic schema fixture is valid JSON")
}
