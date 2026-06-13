use std::collections::HashSet;

use super::{CorpusInput, CorpusProfile, Topic, scale_document_inputs, topic_label};

#[test]
fn tiny_profile_has_four_topics_with_documents_and_queries() {
    let inputs = CorpusProfile::Tiny.inputs();
    let document_count = inputs.iter().filter(|input| input.is_document).count();
    let query_count = inputs.len() - document_count;

    assert_eq!(document_count, 16);
    assert_eq!(query_count, 4);
}

#[test]
fn scaled_ambiguous_profile_combines_ambiguous_and_agent_memory() {
    let scaled = CorpusProfile::ScaledAmbiguousMemory.inputs();
    let expected =
        CorpusProfile::AmbiguousMemory.inputs().len() + CorpusProfile::AgentMemory.inputs().len();

    assert_eq!(scaled.len(), expected);
}

#[test]
fn code_alias_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::CodeAliasMemory, 8);
}

#[test]
fn wide_code_alias_profile_extends_target_queries() {
    assert_targeted_profile(CorpusProfile::CodeAliasWideMemory, 16);
}

#[test]
fn project_code_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectCodeMemory, 16);
}

#[test]
fn project_code_alias_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectCodeAliasMemory, 16);
}

#[test]
fn project_source_code_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectSourceCodeMemory, 16);
}

#[test]
fn project_source_file_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectSourceFileMemory, 8);
}

#[test]
fn project_source_chunk_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectSourceChunkMemory, 16);
}

#[test]
fn project_workspace_source_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectWorkspaceSourceMemory, 16);
}

#[test]
fn project_migration_profile_targets_existing_documents() {
    assert_targeted_profile(CorpusProfile::ProjectMigrationMemory, 16);
}

#[test]
fn project_source_chunk_profile_keeps_graph_roots_target_free() {
    assert_graph_roots_target_free(CorpusProfile::ProjectSourceChunkMemory);
}

#[test]
fn project_workspace_source_profile_keeps_graph_roots_target_free() {
    assert_graph_roots_target_free(CorpusProfile::ProjectWorkspaceSourceMemory);
}

#[test]
fn project_migration_profile_keeps_graph_roots_target_free() {
    assert_graph_roots_target_free(CorpusProfile::ProjectMigrationMemory);
}

#[test]
fn project_migration_profile_contains_current_state_decoys() {
    let inputs = CorpusProfile::ProjectMigrationMemory.inputs();
    let decoys = inputs
        .iter()
        .filter(|input| {
            input.is_document
                && input.target_key.is_none()
                && contains_current_state_negative_marker(input.text())
        })
        .count();

    assert_eq!(decoys, 8);
}

#[test]
fn scaled_document_inputs_repeat_documents_without_duplicating_targets() {
    let inputs = CorpusProfile::ProjectSourceChunkMemory.inputs();
    let document_count = inputs.iter().filter(|input| input.is_document).count();
    let query_count = inputs.len() - document_count;
    let target_count = inputs
        .iter()
        .filter(|input| input.is_document && input.target_key.is_some())
        .count();

    let scaled = scale_document_inputs(inputs, 3);
    let scaled_documents = scaled.iter().filter(|input| input.is_document).count();
    let scaled_queries = scaled.len() - scaled_documents;
    let scaled_targets = scaled
        .iter()
        .filter(|input| input.is_document && input.target_key.is_some())
        .count();
    let duplicate_targets = scaled
        .iter()
        .skip(document_count)
        .filter(|input| input.is_document && input.target_key.is_some())
        .count();
    let duplicate_markers = scaled
        .iter()
        .filter(|input| input.is_document && input.text().contains("[embedding corpus duplicate"))
        .count();

    assert_eq!(scaled_documents, document_count * 3);
    assert_eq!(scaled_queries, query_count);
    assert_eq!(scaled_targets, target_count);
    assert_eq!(duplicate_targets, 0);
    assert_eq!(duplicate_markers, document_count * 2);
    assert_targeted_inputs(&scaled, query_count);
}

#[test]
fn project_workspace_source_profile_reads_current_files() {
    let inputs = CorpusProfile::ProjectWorkspaceSourceMemory.inputs();
    let plan_cache_doc = target_doc(&inputs, "workspace-gql-session-plan-cache");

    assert!(
        plan_cache_doc
            .text()
            .contains("crates/selene-gql/src/runtime/session.rs")
    );
    assert!(plan_cache_doc.text().contains("pub fn with_plan_cache"));
}

#[test]
fn parses_corpus_profile_values() {
    assert!(matches!(
        CorpusProfile::from_value("tiny"),
        CorpusProfile::Tiny
    ));
    assert!(matches!(
        CorpusProfile::from_value("code_alias"),
        CorpusProfile::CodeAliasMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("code_alias_wide"),
        CorpusProfile::CodeAliasWideMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_project_code"),
        CorpusProfile::ProjectCodeMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_project_code_alias"),
        CorpusProfile::ProjectCodeAliasMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_source_code"),
        CorpusProfile::ProjectSourceCodeMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_source_chunk"),
        CorpusProfile::ProjectSourceChunkMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_source_file"),
        CorpusProfile::ProjectSourceFileMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_workspace_source"),
        CorpusProfile::ProjectWorkspaceSourceMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("selene_project_migration"),
        CorpusProfile::ProjectMigrationMemory
    ));
    assert!(matches!(
        CorpusProfile::from_value("scaled_ambiguous_memory"),
        CorpusProfile::ScaledAmbiguousMemory
    ));
}

#[test]
fn topic_labels_are_distinct() {
    let labels = [
        topic_label(Topic::Gql),
        topic_label(Topic::Vector),
        topic_label(Topic::AgentMemory),
        topic_label(Topic::Code),
    ];
    let unique = labels.iter().collect::<HashSet<_>>();

    assert_eq!(unique.len(), labels.len());
}

fn assert_targeted_profile(profile: CorpusProfile, expected_queries: usize) {
    let inputs = profile.inputs();
    assert_targeted_inputs(&inputs, expected_queries);
}

fn assert_targeted_inputs(inputs: &[CorpusInput], expected_queries: usize) {
    let document_keys = inputs
        .iter()
        .filter(|input| input.is_document)
        .filter_map(|input| input.target_key)
        .collect::<HashSet<_>>();
    let query_targets = inputs
        .iter()
        .filter(|input| !input.is_document)
        .map(|input| input.target_key.expect("targeted query has target"))
        .collect::<Vec<_>>();

    assert_eq!(query_targets.len(), expected_queries);
    assert!(
        query_targets
            .iter()
            .all(|target| document_keys.contains(target))
    );
}

fn assert_graph_roots_target_free(profile: CorpusProfile) {
    let inputs = profile.inputs();
    for topic in [Topic::Gql, Topic::Vector, Topic::AgentMemory, Topic::Code] {
        let roots = inputs
            .iter()
            .filter(|input| input.is_document && input.topic == topic)
            .take(2)
            .collect::<Vec<_>>();

        assert_eq!(roots.len(), 2);
        assert!(roots.iter().all(|root| root.target_key.is_none()));
    }
}

fn target_doc<'a>(inputs: &'a [CorpusInput], target: &str) -> &'a CorpusInput {
    inputs
        .iter()
        .find(|input| input.is_document && input.target_key == Some(target))
        .expect("target document exists")
}

fn contains_current_state_negative_marker(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["stale", "superseded", "contradict"]
        .iter()
        .any(|marker| text.contains(marker))
}
