//! M5e pack snapshot corpus harness.

mod common;
mod pack_snapshot_support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use selene_gql::{
    ProcedureError, Session, StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;
use selene_pack::{
    Gate, PackFixtureKind, PackHistorySource, PackSnapshotInput, ProcedurePackRegistry,
    pack_summary,
};
use selene_persist::WalReader;
use selene_testing::{
    GATE_COVERAGE, LIFECYCLE_EVENT_COVERAGE, PackCorpus, PackCorpusCategory, PackCorpusEntry,
    PackCorpusFixture, PackGate, PackManifestFixture,
};

use pack_snapshot_support::lifecycle_runner::run_lifecycle_script;
use pack_snapshot_support::manifest_materializer::materialize_manifest;
use pack_snapshot_support::wal_fixture::{HISTORY_GRAPH_ID, write_history_wal};

#[test]
fn corpus_snapshots_match() {
    for entry in PackCorpus::m5e().entries() {
        let snapshot = execute_entry(entry);
        insta::with_settings!({ snapshot_suffix => entry.slug }, {
            insta::assert_snapshot!(snapshot.to_string());
        });
    }
}

#[test]
fn corpus_slugs_are_unique() {
    let mut slugs = BTreeSet::new();
    for entry in PackCorpus::m5e().entries() {
        assert!(slugs.insert(entry.slug), "duplicate slug {}", entry.slug);
    }
}

#[test]
fn corpus_categories_covered() {
    let actual = PackCorpus::m5e()
        .entries()
        .map(|entry| entry.category)
        .collect::<BTreeSet<_>>();
    let expected = [
        PackCorpusCategory::Manifest,
        PackCorpusCategory::Lifecycle,
        PackCorpusCategory::Hash,
        PackCorpusCategory::History,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn corpus_covers_every_gate() {
    let actual = PackCorpus::m5e()
        .entries()
        .flat_map(|entry| entry.covered_gates.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected = GATE_COVERAGE.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn corpus_covers_every_lifecycle_event_kind() {
    let actual = PackCorpus::m5e()
        .entries()
        .flat_map(|entry| entry.covered_events.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected = LIFECYCLE_EVENT_COVERAGE
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn pack_gate_mirror_matches_selene_pack_gate_all() {
    assert_eq!(PackGate::ALL.len(), Gate::ALL.len());
    for (pack_gate, gate) in PackGate::ALL.iter().copied().zip(Gate::ALL.iter().copied()) {
        assert_eq!(pack_gate.id(), gate.id());
        assert_eq!(pack_gate, gate_to_pack(gate));
    }
}

#[test]
fn error_manifest_fixtures_observed_gates_match_declared() {
    for entry in PackCorpus::m5e().entries() {
        let PackCorpusFixture::ManifestParse { manifest } = &entry.fixture else {
            continue;
        };
        if let Err(error) = materialize_manifest(manifest) {
            let observed = gate_to_pack(error.gate());
            assert!(
                entry.covered_gates.contains(&observed),
                "{} observed gate {:?} not in {:?}",
                entry.slug,
                observed,
                entry.covered_gates
            );
        }
    }
}

fn execute_entry(entry: &PackCorpusEntry) -> selene_pack::PackSnapshot {
    match &entry.fixture {
        PackCorpusFixture::ManifestParse { manifest } => {
            let result = materialize_manifest(manifest);
            pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::ManifestParse {
                    result: result.as_ref(),
                },
            })
        }
        PackCorpusFixture::LifecycleRun { script, sink_mode } => {
            let result = run_lifecycle_script(script, *sink_mode);
            pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::LifecycleRun {
                    events: &result.events,
                    final_registry: &result.registry,
                    error: result.error.as_ref(),
                },
            })
        }
        PackCorpusFixture::HashCanonical { manifest } => {
            let manifest = expect_manifest(manifest, entry.slug);
            pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::HashCanonical {
                    manifest: &manifest,
                },
            })
        }
        PackCorpusFixture::HistoryReplay { entries } => {
            let wal = write_history_wal(entries);
            let rows = execute_history_rows(wal.path().to_path_buf());
            pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::HistoryRows { rows: &rows },
            })
        }
        _ => unreachable!("unknown pack corpus fixture"),
    }
}

fn expect_manifest(
    fixture: &PackManifestFixture,
    slug: &str,
) -> selene_pack::ProcedurePackManifest {
    materialize_manifest(fixture).unwrap_or_else(|error| {
        panic!("{slug} expected a valid manifest, got {error:?}");
    })
}

#[derive(Clone)]
struct PathHistorySource {
    path: Arc<PathBuf>,
}

impl PackHistorySource for PathHistorySource {
    fn open_wal_reader(&self) -> Result<WalReader, ProcedureError> {
        WalReader::open(&self.path).map_err(|source| ProcedureError::Internal {
            detail: format!("pack history WAL open failed: {source}"),
        })
    }
}

fn execute_history_rows(path: PathBuf) -> Vec<Vec<selene_core::Value>> {
    let registry = ProcedurePackRegistry::with_builtins_and_history(Arc::new(PathHistorySource {
        path: Arc::new(path),
    }))
    .expect("platform built-ins register");
    let graph = SharedGraph::new(HISTORY_GRAPH_ID);
    let statement = parse("CALL selene.pack.history() YIELD *").expect("history query parses");
    let analyzed = analyze(statement, &registry, None).expect("history query analyzes");
    let plan = plan(&analyzed, &registry).expect("history query plans");
    let mut session = Session::new(&graph);
    let output = execute_statement(&plan, &mut session, &registry).expect("history executes");
    let StatementOutput::Rows(table) = output else {
        panic!("expected history rows");
    };
    table
        .rows()
        .iter()
        .map(|row| row.values().to_vec())
        .collect()
}

fn gate_to_pack(gate: Gate) -> PackGate {
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
