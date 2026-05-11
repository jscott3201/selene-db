//! M5e pack snapshot corpus harness.

mod common;
mod pack_snapshot_support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use selene_gql::{
    BindingTable, ProcedureError, Session, StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;
use selene_pack::{
    Gate, ManifestError, PackFixtureKind, PackHistorySource, PackSnapshot, PackSnapshotInput,
    ProcedurePackRegistry, pack_summary,
};
use selene_persist::WalReader;
use selene_testing::{
    GATE_COVERAGE, LIFECYCLE_EVENT_COVERAGE, PackCorpus, PackCorpusCategory, PackCorpusEntry,
    PackCorpusFixture, PackGate, PackManifestFixture,
};

use pack_snapshot_support::coverage_helpers::{
    MANIFEST_PATH_GATES, gate_to_pack, history_event_kind, lifecycle_event_kind,
};
use pack_snapshot_support::lifecycle_runner::run_lifecycle_script;
use pack_snapshot_support::manifest_materializer::materialize_manifest;
use pack_snapshot_support::wal_fixture::{HISTORY_GRAPH_ID, write_history_wal};

#[test]
fn corpus_snapshots_match() {
    for entry in PackCorpus::m5e().entries() {
        let snapshot = execute_entry_observed(entry).snapshot;
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
    let mut actual = BTreeSet::new();
    for entry in PackCorpus::m5e().entries() {
        let execution = execute_entry_observed(entry);
        actual.extend(execution.observed_gates.iter().copied());
    }
    let expected = GATE_COVERAGE.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "observed gates differ from GATE_COVERAGE");
}

#[test]
fn corpus_covers_every_lifecycle_event_kind() {
    let mut actual = BTreeSet::new();
    for entry in PackCorpus::m5e().entries() {
        let execution = execute_entry_observed(entry);
        actual.extend(execution.observed_events.iter().copied());
    }
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

struct EntryExecution {
    snapshot: PackSnapshot,
    observed_gates: BTreeSet<PackGate>,
    observed_events: BTreeSet<selene_testing::PackLifecycleEventKind>,
}

fn execute_entry_observed(entry: &PackCorpusEntry) -> EntryExecution {
    match &entry.fixture {
        PackCorpusFixture::ManifestParse { manifest } => {
            let result = materialize_manifest(manifest);
            let observed_gates = observed_manifest_gates(result.as_ref());
            let snapshot = pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::ManifestParse {
                    result: result.as_ref(),
                },
            });
            EntryExecution {
                snapshot,
                observed_gates,
                observed_events: BTreeSet::new(),
            }
        }
        PackCorpusFixture::LifecycleRun { script, sink_mode } => {
            let result = run_lifecycle_script(script, *sink_mode);
            let observed_events = result
                .events
                .iter()
                .map(lifecycle_event_kind)
                .collect::<BTreeSet<_>>();
            let snapshot = pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::LifecycleRun {
                    events: &result.events,
                    final_registry: &result.registry,
                    error: result.error.as_ref(),
                },
            });
            EntryExecution {
                snapshot,
                observed_gates: result.observed_gates,
                observed_events,
            }
        }
        PackCorpusFixture::HashCanonical { manifest } => {
            let manifest = expect_manifest(manifest, entry.slug);
            let snapshot = pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::HashCanonical {
                    manifest: &manifest,
                },
            });
            EntryExecution {
                snapshot,
                observed_gates: MANIFEST_PATH_GATES.iter().copied().collect(),
                observed_events: BTreeSet::new(),
            }
        }
        PackCorpusFixture::HistoryReplay { entries } => {
            let wal = write_history_wal(entries);
            let table = execute_history_table(wal.path().to_path_buf());
            let observed_events = entries
                .iter()
                .filter_map(history_event_kind)
                .collect::<BTreeSet<_>>();
            let snapshot = pack_summary(&PackSnapshotInput {
                fixture_kind: PackFixtureKind::HistoryRows { table: &table },
            });
            EntryExecution {
                snapshot,
                observed_gates: BTreeSet::new(),
                observed_events,
            }
        }
        _ => unreachable!("unknown pack corpus fixture"),
    }
}

fn observed_manifest_gates(
    result: Result<&selene_pack::ProcedurePackManifest, &ManifestError>,
) -> BTreeSet<PackGate> {
    match result {
        Ok(_) => MANIFEST_PATH_GATES.iter().copied().collect(),
        Err(error) => [gate_to_pack(error.gate())].into_iter().collect(),
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

fn execute_history_table(path: PathBuf) -> BindingTable {
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
}
