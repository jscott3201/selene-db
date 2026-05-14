//! Mirror corpus for `selene-vector-pack` procedure invocations.

/// Stable category tag for vector-pack corpus entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VectorPackCorpusCategory {
    /// HNSW read-tier search procedure coverage.
    Search,
    /// HNSW mutation-tier upsert procedure coverage.
    Upsert,
    /// HNSW mutation-tier delete procedure coverage.
    Delete,
    /// HNSW mutation-tier bulk-upsert procedure coverage.
    BulkUpsert,
    /// HNSW mutation-tier bulk-delete procedure coverage.
    BulkDelete,
    /// IVF mutation-tier bulk-upsert procedure coverage.
    IvfBulkUpsert,
    /// IVF mutation-tier bulk-delete procedure coverage.
    IvfBulkDelete,
}

impl VectorPackCorpusCategory {
    /// All declared vector-pack corpus categories.
    pub const ALL: &'static [Self] = &[
        Self::Search,
        Self::Upsert,
        Self::Delete,
        Self::BulkUpsert,
        Self::BulkDelete,
        Self::IvfBulkUpsert,
        Self::IvfBulkDelete,
    ];

    /// Stable category label used by drift tests to anchor exhaustive matching.
    #[must_use]
    pub const fn stable_label(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Upsert => "Upsert",
            Self::Delete => "Delete",
            Self::BulkUpsert => "BulkUpsert",
            Self::BulkDelete => "BulkDelete",
            Self::IvfBulkUpsert => "IvfBulkUpsert",
            Self::IvfBulkDelete => "IvfBulkDelete",
        }
    }
}

const _ASSERT_CATEGORY_ALL_MATCHES_VARIANT_COUNT: () = {
    assert!(VectorPackCorpusCategory::ALL.len() == 7);
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
    /// `vector.upsert`.
    Upsert {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node ID.
        node_id: u64,
        /// Vector components.
        vector: &'static [f32],
    },
    /// `vector.delete`.
    Delete {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node ID.
        node_id: u64,
    },
    /// `vector.bulk_upsert`.
    BulkUpsert {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node IDs.
        node_ids: &'static [u64],
        /// Vector components per node ID.
        vectors: &'static [&'static [f32]],
    },
    /// `vector.bulk_delete`.
    BulkDelete {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node IDs.
        node_ids: &'static [u64],
    },
    /// `vector.ivf_bulk_upsert`.
    IvfBulkUpsert {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node IDs.
        node_ids: &'static [u64],
        /// Vector components per node ID.
        vectors: &'static [&'static [f32]],
    },
    /// `vector.ivf_bulk_delete`.
    IvfBulkDelete {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Source graph node IDs.
        node_ids: &'static [u64],
    },
}

impl VectorPackInvocation {
    /// Return the canonical procedure name path for this invocation.
    #[must_use]
    pub const fn procedure_name(&self) -> &'static [&'static str] {
        match self {
            Self::Search { .. } => &["vector", "search"],
            Self::Upsert { .. } => &["vector", "upsert"],
            Self::Delete { .. } => &["vector", "delete"],
            Self::BulkUpsert { .. } => &["vector", "bulk_upsert"],
            Self::BulkDelete { .. } => &["vector", "bulk_delete"],
            Self::IvfBulkUpsert { .. } => &["vector", "ivf_bulk_upsert"],
            Self::IvfBulkDelete { .. } => &["vector", "ivf_bulk_delete"],
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
            Self::Upsert {
                index_name,
                node_id,
                vector,
            } => format!(
                "CALL vector.upsert({}, {node_id}, {})",
                quoted(index_name),
                f32_list(vector)
            ),
            Self::Delete {
                index_name,
                node_id,
            } => format!("CALL vector.delete({}, {node_id})", quoted(index_name)),
            Self::BulkUpsert {
                index_name,
                node_ids,
                vectors,
            } => format!(
                "CALL vector.bulk_upsert({}, {}, {})",
                quoted(index_name),
                u64_list(node_ids),
                f32_matrix(vectors)
            ),
            Self::BulkDelete {
                index_name,
                node_ids,
            } => format!(
                "CALL vector.bulk_delete({}, {})",
                quoted(index_name),
                u64_list(node_ids)
            ),
            Self::IvfBulkUpsert {
                index_name,
                node_ids,
                vectors,
            } => format!(
                "CALL vector.ivf_bulk_upsert({}, {}, {})",
                quoted(index_name),
                u64_list(node_ids),
                f32_matrix(vectors)
            ),
            Self::IvfBulkDelete {
                index_name,
                node_ids,
            } => format!(
                "CALL vector.ivf_bulk_delete({}, {})",
                quoted(index_name),
                u64_list(node_ids)
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

    /// Construct the B2 seed corpus, preserving B1 entries in order.
    #[must_use]
    pub fn b2_seed() -> Self {
        let mut corpus = Self::b1_seed();
        corpus.entries.extend([
            VectorPackCorpusEntry {
                name: "upsert_default",
                category: VectorPackCorpusCategory::Upsert,
                invocation: VectorPackInvocation::Upsert {
                    index_name: "default",
                    node_id: 42,
                    vector: &[1.0, 0.0, 0.0, 0.0],
                },
            },
            VectorPackCorpusEntry {
                name: "delete_default",
                category: VectorPackCorpusCategory::Delete,
                invocation: VectorPackInvocation::Delete {
                    index_name: "default",
                    node_id: 42,
                },
            },
        ]);
        corpus
    }

    /// Construct the B3 seed corpus, preserving B2 entries in order.
    #[must_use]
    pub fn b3_seed() -> Self {
        let mut corpus = Self::b2_seed();
        corpus.entries.extend([
            VectorPackCorpusEntry {
                name: "bulk_upsert_default",
                category: VectorPackCorpusCategory::BulkUpsert,
                invocation: VectorPackInvocation::BulkUpsert {
                    index_name: "default",
                    node_ids: &[42, 43],
                    vectors: &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
                },
            },
            VectorPackCorpusEntry {
                name: "bulk_delete_default",
                category: VectorPackCorpusCategory::BulkDelete,
                invocation: VectorPackInvocation::BulkDelete {
                    index_name: "default",
                    node_ids: &[42, 43],
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_bulk_upsert_default",
                category: VectorPackCorpusCategory::IvfBulkUpsert,
                invocation: VectorPackInvocation::IvfBulkUpsert {
                    index_name: "default",
                    node_ids: &[42, 43],
                    vectors: &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_bulk_delete_default",
                category: VectorPackCorpusCategory::IvfBulkDelete,
                invocation: VectorPackInvocation::IvfBulkDelete {
                    index_name: "default",
                    node_ids: &[42, 43],
                },
            },
        ]);
        corpus
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

fn u64_list(values: &[u64]) -> String {
    let rendered = values.iter().map(u64::to_string).collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

fn f32_matrix(rows: &[&[f32]]) -> String {
    let rendered = rows.iter().map(|row| f32_list(row)).collect::<Vec<_>>();
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
    fn procedure_name_returns_static_slice_for_upsert_variant() {
        let invocation = VectorPackInvocation::Upsert {
            index_name: "default",
            node_id: 1,
            vector: &[1.0],
        };

        assert_eq!(invocation.procedure_name(), &["vector", "upsert"]);
    }

    #[test]
    fn procedure_name_returns_static_slice_for_delete_variant() {
        let invocation = VectorPackInvocation::Delete {
            index_name: "default",
            node_id: 1,
        };

        assert_eq!(invocation.procedure_name(), &["vector", "delete"]);
    }

    #[test]
    fn procedure_name_returns_static_slice_for_bulk_variants() {
        let invocations = [
            (
                VectorPackInvocation::BulkUpsert {
                    index_name: "default",
                    node_ids: &[1],
                    vectors: &[&[1.0]],
                },
                &["vector", "bulk_upsert"][..],
            ),
            (
                VectorPackInvocation::BulkDelete {
                    index_name: "default",
                    node_ids: &[1],
                },
                &["vector", "bulk_delete"][..],
            ),
            (
                VectorPackInvocation::IvfBulkUpsert {
                    index_name: "default",
                    node_ids: &[1],
                    vectors: &[&[1.0]],
                },
                &["vector", "ivf_bulk_upsert"][..],
            ),
            (
                VectorPackInvocation::IvfBulkDelete {
                    index_name: "default",
                    node_ids: &[1],
                },
                &["vector", "ivf_bulk_delete"][..],
            ),
        ];

        for (invocation, expected) in invocations {
            assert_eq!(invocation.procedure_name(), expected);
        }
    }

    #[test]
    fn procedure_name_covers_all_variants() {
        let invocations = [
            VectorPackInvocation::Search {
                index_name: "default",
                query: &[1.0],
                k: 1,
                ef_search: None,
                filter_nodes: &[],
            },
            VectorPackInvocation::Upsert {
                index_name: "default",
                node_id: 1,
                vector: &[1.0],
            },
            VectorPackInvocation::Delete {
                index_name: "default",
                node_id: 1,
            },
            VectorPackInvocation::BulkUpsert {
                index_name: "default",
                node_ids: &[1],
                vectors: &[&[1.0]],
            },
            VectorPackInvocation::BulkDelete {
                index_name: "default",
                node_ids: &[1],
            },
            VectorPackInvocation::IvfBulkUpsert {
                index_name: "default",
                node_ids: &[1],
                vectors: &[&[1.0]],
            },
            VectorPackInvocation::IvfBulkDelete {
                index_name: "default",
                node_ids: &[1],
            },
        ];

        let mut names = BTreeSet::new();
        for invocation in invocations {
            let name = invocation.procedure_name();
            assert_eq!(name.len(), 2);
            assert_eq!(name[0], "vector");
            assert!(names.insert(name));
        }
        assert_eq!(names.len(), 7);
    }

    #[test]
    fn category_all_matches_exhaustive_anchor() {
        let declared = VectorPackCorpusCategory::ALL
            .iter()
            .map(|category| category.stable_label())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            declared,
            [
                "Search",
                "Upsert",
                "Delete",
                "BulkUpsert",
                "BulkDelete",
                "IvfBulkUpsert",
                "IvfBulkDelete",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
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

    #[test]
    fn corpus_b1_seed_unchanged_inside_b2_seed() {
        let b1 = VectorPackCorpus::b1_seed();
        let b2 = VectorPackCorpus::b2_seed();

        assert_eq!(&b2.entries()[..b1.entries().len()], b1.entries());
    }

    #[test]
    fn corpus_b2_seed_covers_all_declared_procedures() {
        let names = VectorPackCorpus::b2_seed()
            .entries()
            .iter()
            .map(|entry| entry.invocation.procedure_name())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            names,
            [
                &["vector", "search"][..],
                &["vector", "upsert"][..],
                &["vector", "delete"][..],
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn corpus_b2_seed_unchanged_inside_b3_seed() {
        let b2 = VectorPackCorpus::b2_seed();
        let b3 = VectorPackCorpus::b3_seed();

        assert_eq!(&b3.entries()[..b2.entries().len()], b2.entries());
    }

    #[test]
    fn corpus_b3_seed_covers_all_declared_procedures() {
        let names = VectorPackCorpus::b3_seed()
            .entries()
            .iter()
            .map(|entry| entry.invocation.procedure_name())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            names,
            [
                &["vector", "search"][..],
                &["vector", "upsert"][..],
                &["vector", "delete"][..],
                &["vector", "bulk_upsert"][..],
                &["vector", "bulk_delete"][..],
                &["vector", "ivf_bulk_upsert"][..],
                &["vector", "ivf_bulk_delete"][..],
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
    }
}
