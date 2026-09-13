//! Compiled conformance evidence and release-claim harness.

mod cli;
mod run;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use selene_db::{CreatePolicy, Database, ErrorKind, ExecutionOutcome, ObjectPath, SchemaPath};
use selene_gql::{GqlStatus, ParserError};
use selene_profile::{
    EvidenceDisposition, EvidenceRecord, RuleRecord, ValidatedConformance, ValidatedProfile,
    load_conformance, load_profile,
};

use run::{Actual, ObservedSideEffects, ObservedStatus};

const PROFILE_PATH: &str = "spec/gql-profile/profile.json";

type Runner = fn(&str) -> Result<Actual, String>;

#[derive(Clone, Copy, Debug)]
struct Registration {
    id: &'static str,
    evidence_id: &'static str,
    fixture_path: &'static str,
    fixture: &'static str,
    required_fragment: &'static str,
    runner_name: &'static str,
    runner: Runner,
}

#[derive(Clone, Copy, Debug)]
struct PendingMarker {
    evidence_id: &'static str,
}

macro_rules! registration {
    ($id:literal, $evidence:literal, $fixture:literal, $fragment:literal, $runner:path) => {
        Registration {
            id: $id,
            evidence_id: $evidence,
            fixture_path: concat!("crates/selene-testing/", $fixture),
            fixture: include_str!(concat!("../../", $fixture)),
            required_fragment: $fragment,
            runner_name: stringify!($runner),
            runner: $runner,
        }
    };
}

const REGISTRATIONS: &[Registration] = &[
    registration!(
        "REG-G010-POSITIVE",
        "EVID-CONFORMANCE-G010-POSITIVE",
        "corpus/positive/G010-walk-explicit.gql",
        "MATCH WALK (n:Person) RETURN n",
        run_g010_positive
    ),
    registration!(
        "REG-GC04-NEGATIVE",
        "EVID-CONFORMANCE-GC04-NEGATIVE",
        "corpus/negative/GG05-create-graph-copy-of.gql",
        "AS COPY OF",
        run_gc04_negative
    ),
    registration!(
        "REG-GC04-POSITIVE",
        "EVID-CONFORMANCE-GC04-POSITIVE",
        "corpus/positive/GC05-create-graph-if-not-exists.gql",
        "CREATE GRAPH IF NOT EXISTS demo ANY",
        run_gc04_positive
    ),
    registration!(
        "REG-GC04-STATUS",
        "EVID-CONFORMANCE-GC04-STATUS",
        "corpus/positive/GC04-create-graph-open-type.gql",
        "CREATE GRAPH /memory/episodes ANY",
        run_gc04_status
    ),
];

const PENDING_MARKERS: &[PendingMarker] = &[PendingMarker {
    evidence_id: "EVID-CONFORMANCE-INVENTORY-PENDING",
}];

#[derive(Debug, thiserror::Error)]
enum ConformanceError {
    #[error(transparent)]
    Profile(#[from] selene_profile::ProfileError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("conformance harness: {0}")]
    Invalid(String),
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

#[derive(Clone, Debug)]
struct Contract {
    registration: Registration,
    evidence: EvidenceRecord,
    rules: Vec<RuleRecord>,
}

#[derive(Debug)]
struct Harness {
    profile: ValidatedProfile,
    registry: ValidatedConformance,
    contracts: Vec<Contract>,
    markers: Vec<PendingMarker>,
}

impl Harness {
    fn load(root: &Path) -> Result<Self, ConformanceError> {
        let profile = load_profile(&root.join(PROFILE_PATH))?;
        let registry = load_conformance(root, &profile)?;
        Self::from_parts(profile, registry, REGISTRATIONS, PENDING_MARKERS)
    }

    fn from_parts(
        profile: ValidatedProfile,
        registry: ValidatedConformance,
        registrations: &[Registration],
        markers: &[PendingMarker],
    ) -> Result<Self, ConformanceError> {
        let evidence = registry
            .evidence()
            .evidence
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let rules = registry
            .rules()
            .rules
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let mut registration_ids = BTreeSet::new();
        let mut evidence_ids = BTreeSet::new();
        let mut contracts = Vec::new();
        for registration in registrations {
            if !registration_ids.insert(registration.id)
                || !evidence_ids.insert(registration.evidence_id)
            {
                return Err(invalid("duplicate compiled registration or evidence ID"));
            }
            let static_record = evidence.get(registration.evidence_id).ok_or_else(|| {
                invalid(format!(
                    "unknown compiled evidence {}",
                    registration.evidence_id
                ))
            })?;
            if static_record.registration.as_deref() != Some(registration.id)
                || !matches!(static_record.disposition, EvidenceDisposition::Complete)
            {
                return Err(invalid(format!(
                    "compiled registration {} contradicts static evidence",
                    registration.id
                )));
            }
            if registration.required_fragment.is_empty()
                || !registration
                    .fixture
                    .contains(registration.required_fragment)
            {
                return Err(invalid(format!(
                    "{} has a stale fixture fragment",
                    registration.id
                )));
            }
            if registration.fixture_path.is_empty() || registration.runner_name.is_empty() {
                return Err(invalid(format!(
                    "{} has incomplete compiled metadata",
                    registration.id
                )));
            }
            contracts.push(Contract {
                registration: *registration,
                evidence: (*static_record).clone(),
                rules: static_record
                    .targets
                    .iter()
                    .map(|target| (*rules[&target.rule_id.as_str()]).clone())
                    .collect(),
            });
        }
        for record in evidence.values() {
            if matches!(record.disposition, EvidenceDisposition::Complete)
                && !evidence_ids.contains(record.id.as_str())
            {
                return Err(invalid(format!(
                    "complete evidence {} has no compiled registration",
                    record.id
                )));
            }
        }
        let mut marker_ids = BTreeSet::new();
        for marker in markers {
            if !marker_ids.insert(marker.evidence_id) {
                return Err(invalid("duplicate compiled pending marker"));
            }
            let record = evidence
                .get(marker.evidence_id)
                .ok_or_else(|| invalid(format!("unknown pending marker {}", marker.evidence_id)))?;
            if record.registration.is_some()
                || !matches!(record.disposition, EvidenceDisposition::Pending { .. })
            {
                return Err(invalid(format!(
                    "pending marker {} contradicts static evidence",
                    marker.evidence_id
                )));
            }
        }
        for record in evidence.values() {
            if matches!(record.disposition, EvidenceDisposition::Pending { .. })
                && !marker_ids.contains(record.id.as_str())
            {
                return Err(invalid(format!(
                    "pending evidence {} has no compiled marker",
                    record.id
                )));
            }
        }
        contracts.sort_by_key(|item| item.registration.id);
        let mut markers = markers.to_vec();
        markers.sort_by_key(|item| item.evidence_id);
        Ok(Self {
            profile,
            registry,
            contracts,
            markers,
        })
    }
}

fn invalid(message: impl Into<String>) -> ConformanceError {
    ConformanceError::Invalid(message.into())
}

fn run_g010_positive(source: &str) -> Result<Actual, String> {
    let statement = selene_gql::parse(source).map_err(|error| error.to_string())?;
    let features = selene_gql::feature_walk(&statement)
        .into_iter()
        .map(|item| item.feature_id)
        .collect::<BTreeSet<_>>();
    if features != BTreeSet::from([selene_profile::FeatureId::G010]) {
        return Err("G010 WALK feature observation did not match".to_owned());
    }
    Ok(Actual::parser(ObservedStatus::Success))
}

/// The `<graph source>` clause is rejected before planning with feature GG05
/// (ISO/IEC 39075:2024 section 12.4 CR7); nothing reaches the catalog.
fn run_gc04_negative(source: &str) -> Result<Actual, String> {
    let error = match selene_gql::parse(source) {
        Ok(_) => return Err("graph source clause unexpectedly parsed".to_owned()),
        Err(error) => error,
    };
    let feature_id = match error {
        ParserError::UnsupportedFeature { feature_id, .. } => feature_id,
        _ => return Err("graph source clause did not return UnsupportedFeature".to_owned()),
    };
    if feature_id != selene_profile::FeatureId::GG05 {
        return Err("unsupported feature was not GG05".to_owned());
    }
    Ok(Actual::parser(ObservedStatus::Error {
        gqlstatus: GqlStatus::FEATURE_NOT_SUPPORTED.as_str().to_owned(),
    }))
}

/// `CREATE GRAPH IF NOT EXISTS demo ANY` through the database facade completes
/// with `00001` and publishes the graph in the current working schema.
fn run_gc04_positive(source: &str) -> Result<Actual, String> {
    let database = Database::builder().build();
    let memory = SchemaPath::regular("selene", "memory").map_err(|error| error.to_string())?;
    let selected =
        ObjectPath::regular("selene", "memory", "evidence").map_err(|error| error.to_string())?;
    database
        .catalog()
        .create_schema(&memory, CreatePolicy::Strict)
        .map_err(|error| error.to_string())?;
    database
        .catalog()
        .create_graph(&selected, None, CreatePolicy::Strict)
        .map_err(|error| error.to_string())?;
    let before = database.catalog().snapshot().generation();
    let outcome = database
        .session(&selected)
        .map_err(|error| error.to_string())?
        .execute(source)
        .map_err(|error| error.to_string())?;
    if outcome != ExecutionOutcome::SUCCESSFUL_OMITTED {
        return Err(format!(
            "CREATE GRAPH did not complete with 00001: {outcome:?}"
        ));
    }
    let snapshot = database.catalog().snapshot();
    if snapshot.generation() <= before {
        return Err("CREATE GRAPH published no catalog generation".to_owned());
    }
    let created = snapshot
        .graphs(&memory)
        .map_err(|error| error.to_string())?
        .into_iter()
        .any(|graph| graph.path.object().canonical() == "demo");
    if !created {
        return Err("CREATE GRAPH did not publish /selene/memory/demo".to_owned());
    }
    Ok(Actual::executed(
        ObservedStatus::Success,
        ObservedSideEffects::Required,
    ))
}

/// A strict duplicate `CREATE GRAPH` through the database facade reports
/// `42N10` and publishes nothing.
fn run_gc04_status(source: &str) -> Result<Actual, String> {
    let database = Database::builder().build();
    let memory = SchemaPath::regular("selene", "memory").map_err(|error| error.to_string())?;
    database
        .catalog()
        .create_schema(&memory, CreatePolicy::Strict)
        .map_err(|error| error.to_string())?;
    let selected =
        ObjectPath::regular("selene", "memory", "evidence").map_err(|error| error.to_string())?;
    database
        .catalog()
        .create_graph(&selected, None, CreatePolicy::Strict)
        .map_err(|error| error.to_string())?;
    let session = database
        .session(&selected)
        .map_err(|error| error.to_string())?;
    session.execute(source).map_err(|error| error.to_string())?;
    let before = database.catalog().snapshot();
    let error = match session.execute(source) {
        Ok(outcome) => return Err(format!("duplicate CREATE GRAPH succeeded: {outcome:?}")),
        Err(error) => error,
    };
    if error.kind() != ErrorKind::CatalogObjectAlreadyExists {
        return Err(format!(
            "duplicate CREATE GRAPH kind was {:?}",
            error.kind()
        ));
    }
    let Some(status) = error.gqlstatus() else {
        return Err("duplicate CREATE GRAPH carried no GQLSTATUS".to_owned());
    };
    if status != selene_db::GqlStatus::DUPLICATE_OBJECT {
        return Err(format!("duplicate CREATE GRAPH status was {status}"));
    }
    if !database.catalog().snapshot().shares_state_with(&before) {
        return Err("duplicate CREATE GRAPH published catalog state".to_owned());
    }
    Ok(Actual::executed(
        ObservedStatus::Error {
            gqlstatus: status.as_str().to_owned(),
        },
        ObservedSideEffects::Forbidden,
    ))
}

/// Run the command-line conformance runner or traceability generator.
///
/// # Errors
///
/// Returns an error for invalid arguments, registry drift, failed evidence, a
/// denied claim, or an output path inside the repository.
pub fn main_cli() -> Result<(), Box<dyn Error>> {
    cli::run().map_err(Into::into)
}
