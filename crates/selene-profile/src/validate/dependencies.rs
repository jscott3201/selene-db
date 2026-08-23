//! Runtime and claim consistency across feature implications.

use std::collections::{BTreeMap, BTreeSet};

use crate::closure::{ClosureGraph, DependencyRelation};
use crate::{ClaimState, FeatureRecord, Profile, RuntimeSupport};

use super::{ProfileError, invalid};

pub(super) fn validate(profile: &Profile, graph: &ClosureGraph) -> Result<(), ProfileError> {
    let features = profile
        .features
        .iter()
        .map(|feature| (feature.id.as_str(), feature))
        .collect::<BTreeMap<_, _>>();
    let selected = profile
        .selected_features
        .iter()
        .map(|feature| feature.as_str())
        .collect::<BTreeSet<_>>();

    for feature in &profile.features {
        if feature.runtime_support == RuntimeSupport::Supported
            && !selected.contains(feature.id.as_str())
        {
            return Err(invalid(format!(
                "runtime-supported feature {} is omitted from selected_features",
                feature.id.as_str()
            )));
        }
        if feature.runtime_support == RuntimeSupport::Supported {
            validate_runtime_dependencies(feature, &features, graph)?;
        }
        if feature.claim_state == ClaimState::Claimed {
            require_complete_evidence(feature, "claimed feature")?;
            validate_claim_dependencies(feature, &features, graph)?;
        }
    }

    if profile.release_claimable {
        for id in graph.closure_for(profile.selected_features.iter().map(|id| id.as_str())) {
            let feature = features[id.as_str()];
            if feature.claim_state != ClaimState::Claimed || feature.evidence.is_empty() {
                let path = graph
                    .shortest_path_from(
                        profile.selected_features.iter().map(|item| item.as_str()),
                        &id,
                    )
                    .expect("target closure members have a selected path");
                let relation = if selected.contains(id.as_str()) {
                    "directly selected"
                } else if path.len() == 2 {
                    "direct dependency"
                } else {
                    "transitive dependency"
                };
                return Err(invalid(format!(
                    "release_claimable profile {} has incomplete {relation} {id}: claim={:?}, evidence={}; path {}",
                    profile.profile_id,
                    feature.claim_state,
                    evidence_state(feature),
                    path.join(" -> ")
                )));
            }
        }
    }
    Ok(())
}

fn validate_runtime_dependencies(
    source: &FeatureRecord,
    features: &BTreeMap<&str, &FeatureRecord>,
    graph: &ClosureGraph,
) -> Result<(), ProfileError> {
    for dependency in graph.dependencies(source.id.as_str()) {
        let target = features[dependency];
        if target.runtime_support != RuntimeSupport::Supported {
            return Err(invalid(format!(
                "runtime-supported source {} has {} dependency {} with runtime support {:?}; path {}",
                source.id.as_str(),
                relation_name(graph, source.id.as_str(), dependency),
                dependency,
                target.runtime_support,
                graph
                    .shortest_path(source.id.as_str(), dependency)
                    .expect("reachable dependency has a path")
                    .join(" -> ")
            )));
        }
    }
    Ok(())
}

fn validate_claim_dependencies(
    source: &FeatureRecord,
    features: &BTreeMap<&str, &FeatureRecord>,
    graph: &ClosureGraph,
) -> Result<(), ProfileError> {
    for dependency in graph.dependencies(source.id.as_str()) {
        let target = features[dependency];
        if target.claim_state != ClaimState::Claimed || target.evidence.is_empty() {
            return Err(invalid(format!(
                "claimed source {} has incomplete {} dependency {}: claim={:?}, evidence={}; path {}",
                source.id.as_str(),
                relation_name(graph, source.id.as_str(), dependency),
                dependency,
                target.claim_state,
                evidence_state(target),
                graph
                    .shortest_path(source.id.as_str(), dependency)
                    .expect("reachable dependency has a path")
                    .join(" -> ")
            )));
        }
    }
    Ok(())
}

fn require_complete_evidence(feature: &FeatureRecord, kind: &str) -> Result<(), ProfileError> {
    if feature.evidence.is_empty() {
        Err(invalid(format!(
            "{kind} {} has incomplete evidence",
            feature.id.as_str()
        )))
    } else {
        Ok(())
    }
}

fn relation_name(graph: &ClosureGraph, source: &str, target: &str) -> &'static str {
    match graph.relation(source, target) {
        Some(DependencyRelation::Direct) => "direct",
        Some(DependencyRelation::Transitive) => "transitive",
        None => unreachable!("caller passes a reachable dependency"),
    }
}

pub(crate) fn evidence_state(feature: &FeatureRecord) -> &'static str {
    if feature.evidence.is_empty() {
        "incomplete"
    } else {
        "present"
    }
}
