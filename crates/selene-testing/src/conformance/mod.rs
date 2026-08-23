//! Compiled conformance evidence and release-claim harness.

mod cli;
mod run;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use selene_gql::{GqlStatus, ParserError};
use selene_profile::{
    EvidenceDisposition, EvidenceRecord, RuleRecord, ValidatedConformance, ValidatedProfile,
    load_conformance, load_profile,
};

use run::{Actual, ObservedStatus};

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
        "corpus/negative/GC04-graph-management.gql",
        "CREATE GRAPH IF NOT EXISTS demo",
        run_gc04_negative
    ),
    registration!(
        "REG-GC04-STATUS",
        "EVID-CONFORMANCE-GC04-STATUS",
        "corpus/negative/GC04-graph-management.gql",
        "CREATE GRAPH IF NOT EXISTS demo",
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

fn run_gc04_negative(source: &str) -> Result<Actual, String> {
    let error = match selene_gql::parse(source) {
        Ok(_) => return Err("GC04 unexpectedly parsed".to_owned()),
        Err(error) => error,
    };
    let feature_id = match error {
        ParserError::UnsupportedFeature { feature_id, .. } => feature_id,
        _ => return Err("GC04 did not return UnsupportedFeature".to_owned()),
    };
    if feature_id != selene_profile::FeatureId::GC04 {
        return Err("unsupported feature was not GC04".to_owned());
    }
    Ok(Actual::parser(ObservedStatus::Error {
        gqlstatus: GqlStatus::FEATURE_NOT_SUPPORTED.as_str().to_owned(),
    }))
}

fn run_gc04_status(source: &str) -> Result<Actual, String> {
    let error = match selene_gql::parse(source) {
        Ok(_) => return Err("GC04 unexpectedly parsed".to_owned()),
        Err(error) => error,
    };
    if error.gqlstatus() != GqlStatus::FEATURE_NOT_SUPPORTED {
        return Err("GC04 did not return exact GQLSTATUS 42N01".to_owned());
    }
    Ok(Actual::parser(ObservedStatus::Error {
        gqlstatus: error.gqlstatus().as_str().to_owned(),
    }))
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
