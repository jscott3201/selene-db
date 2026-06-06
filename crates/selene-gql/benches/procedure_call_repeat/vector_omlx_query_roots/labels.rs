use selene_testing::local_omlx::CorpusProfile;

pub(super) fn model_id(model: &str) -> String {
    model
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn corpus_label(corpus: CorpusProfile) -> &'static str {
    match corpus {
        CorpusProfile::Tiny => "tiny",
        CorpusProfile::AgentMemory => "agent_memory",
        CorpusProfile::AmbiguousMemory => "ambiguous_memory",
        CorpusProfile::ScaledAmbiguousMemory => "scaled_ambiguous_memory",
        CorpusProfile::CodeAliasMemory => "code_alias_memory",
        CorpusProfile::CodeAliasWideMemory => "code_alias_wide_memory",
        CorpusProfile::ProjectCodeMemory => "project_code_memory",
        CorpusProfile::ProjectCodeAliasMemory => "project_code_alias_memory",
        CorpusProfile::ProjectSourceCodeMemory => "project_source_code_memory",
        CorpusProfile::ProjectSourceChunkMemory => "project_source_chunk_memory",
        CorpusProfile::ProjectSourceFileMemory => "project_source_file_memory",
        CorpusProfile::ProjectWorkspaceSourceMemory => "project_workspace_source_memory",
    }
}

pub(super) fn append_target_hit(mut label: String, target_hit: Option<usize>) -> String {
    if let Some(target_hit) = target_hit {
        label.push_str(&format!("_hitbp{target_hit}"));
    }
    label
}
