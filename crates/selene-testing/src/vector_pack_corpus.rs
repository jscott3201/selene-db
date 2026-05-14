//! Mirror corpus for `selene-vector-pack` procedure invocations.

/// Stable category tag for vector-pack corpus entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VectorPackCorpusCategory {
    /// HNSW read-tier search procedure coverage.
    Search,
}

impl VectorPackCorpusCategory {
    /// All declared vector-pack corpus categories.
    pub const ALL: &'static [Self] = &[Self::Search];

    /// Stable category label used by drift tests to anchor exhaustive matching.
    #[must_use]
    pub const fn stable_label(self) -> &'static str {
        match self {
            Self::Search => "Search",
        }
    }
}

const _ASSERT_CATEGORY_ALL_MATCHES_VARIANT_COUNT: () = {
    assert!(VectorPackCorpusCategory::ALL.len() == 1);
};

/// Procedure invocation mirrored by the vector-pack corpus.
#[derive(Clone, Debug, PartialEq)]
pub enum VectorPackInvocation {
    /// `vector.search`.
    Search {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Query vector components.
        query: &'static [f32],
        /// Requested top-k result count.
        k: usize,
        /// Nullable search-width override.
        ef_search: Option<usize>,
        /// Optional node-filter IDs. Empty means NULL.
        filter_nodes: &'static [u64],
    },
}

impl VectorPackInvocation {
    /// Return the canonical procedure name path for this invocation.
    #[must_use]
    pub const fn procedure_name(&self) -> &'static [&'static str] {
        match self {
            Self::Search { .. } => &["vector", "search"],
        }
    }

    /// Render a deterministic GQL `CALL` string for snapshot testing.
    #[must_use]
    pub fn render_call(&self) -> String {
        match self {
            Self::Search {
                index_name,
                query,
                k,
                ef_search,
                filter_nodes,
            } => format!(
                "CALL vector.search({}, {}, {k}, {}, {})",
                quoted(index_name),
                f32_list(query),
                nullable_usize(*ef_search),
                node_filter(filter_nodes)
            ),
        }
    }
}

/// One vector-pack corpus entry.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPackCorpusEntry {
    /// Stable entry name.
    pub name: &'static str,
    /// Category for coverage accounting.
    pub category: VectorPackCorpusCategory,
    /// Mirrored invocation.
    pub invocation: VectorPackInvocation,
}

impl VectorPackCorpusEntry {
    /// Render this entry to a deterministic single-line snapshot row.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{} [{:?}] {}\n",
            self.name,
            self.category,
            self.invocation.render_call()
        )
    }
}

/// Corpus mirror used by vector-pack tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorPackCorpus {
    entries: Vec<VectorPackCorpusEntry>,
}

impl VectorPackCorpus {
    /// Construct the B1 seed corpus.
    #[must_use]
    pub fn b1_seed() -> Self {
        Self {
            entries: vec![VectorPackCorpusEntry {
                name: "search_default",
                category: VectorPackCorpusCategory::Search,
                invocation: VectorPackInvocation::Search {
                    index_name: "default",
                    query: &[1.0, 0.0, 0.0, 0.0],
                    k: 10,
                    ef_search: None,
                    filter_nodes: &[],
                },
            }],
        }
    }

    /// Borrow corpus entries in deterministic order.
    #[must_use]
    pub fn entries(&self) -> &[VectorPackCorpusEntry] {
        &self.entries
    }

    /// Render the corpus to a deterministic snapshot string.
    #[must_use]
    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(VectorPackCorpusEntry::render)
            .collect()
    }
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

fn nullable_usize(value: Option<usize>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| value.to_string())
}

fn f32_list(values: &[f32]) -> String {
    let rendered = values
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

fn node_filter(nodes: &[u64]) -> String {
    if nodes.is_empty() {
        return "NULL".to_string();
    }
    let rendered = nodes.iter().map(u64::to_string).collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{VectorPackCorpus, VectorPackCorpusCategory, VectorPackInvocation};

    #[test]
    fn procedure_name_returns_static_slice_for_search_variant() {
        let invocation = VectorPackInvocation::Search {
            index_name: "default",
            query: &[1.0],
            k: 1,
            ef_search: None,
            filter_nodes: &[],
        };

        assert_eq!(invocation.procedure_name(), &["vector", "search"]);
    }

    #[test]
    fn procedure_name_covers_all_variants() {
        let invocations = [VectorPackInvocation::Search {
            index_name: "default",
            query: &[1.0],
            k: 1,
            ef_search: None,
            filter_nodes: &[],
        }];

        let mut names = BTreeSet::new();
        for invocation in invocations {
            let name = invocation.procedure_name();
            assert_eq!(name.len(), 2);
            assert_eq!(name[0], "vector");
            assert!(names.insert(name));
        }
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn category_all_matches_exhaustive_anchor() {
        let declared = VectorPackCorpusCategory::ALL
            .iter()
            .map(|category| category.stable_label())
            .collect::<BTreeSet<_>>();

        assert_eq!(declared, ["Search"].into_iter().collect::<BTreeSet<_>>());
    }

    #[test]
    fn entry_render_matches_corpus_render_line() {
        let corpus = VectorPackCorpus::b1_seed();
        let rendered_entries = corpus
            .entries()
            .iter()
            .map(super::VectorPackCorpusEntry::render)
            .collect::<String>();

        assert_eq!(corpus.render(), rendered_entries);
    }
}
