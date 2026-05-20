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
    /// Vector catalog lifecycle create procedure coverage.
    CreateIndex,
    /// Vector catalog lifecycle drop procedure coverage.
    DropIndex,
    /// Vector catalog read-tier listing procedure coverage.
    ListIndexes,
    /// IVF mutation-tier bulk-upsert procedure coverage.
    IvfBulkUpsert,
    /// IVF mutation-tier bulk-delete procedure coverage.
    IvfBulkDelete,
    /// IVF read-tier search procedure coverage.
    IvfSearch,
    /// IVF read-tier stats procedure coverage.
    IvfStats,
}

impl VectorPackCorpusCategory {
    /// All declared vector-pack corpus categories.
    pub const ALL: &'static [Self] = &[
        Self::Search,
        Self::Upsert,
        Self::Delete,
        Self::BulkUpsert,
        Self::BulkDelete,
        Self::CreateIndex,
        Self::DropIndex,
        Self::ListIndexes,
        Self::IvfBulkUpsert,
        Self::IvfBulkDelete,
        Self::IvfSearch,
        Self::IvfStats,
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
            Self::CreateIndex => "CreateIndex",
            Self::DropIndex => "DropIndex",
            Self::ListIndexes => "ListIndexes",
            Self::IvfBulkUpsert => "IvfBulkUpsert",
            Self::IvfBulkDelete => "IvfBulkDelete",
            Self::IvfSearch => "IvfSearch",
            Self::IvfStats => "IvfStats",
        }
    }
}

const _ASSERT_CATEGORY_ALL_MATCHES_VARIANT_COUNT: () = {
    assert!(VectorPackCorpusCategory::ALL.len() == 12);
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
    /// `vector.create_index`.
    CreateIndex {
        /// Vector index name.
        name: &'static str,
        /// Vector index kind (`hnsw` or `ivf`).
        kind: &'static str,
        /// Open-record config literal.
        config: &'static str,
    },
    /// `vector.drop_index`.
    DropIndex {
        /// Vector index name.
        name: &'static str,
    },
    /// `vector.list_indexes`.
    ListIndexes,
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
    /// `vector.ivf_search`.
    IvfSearch {
        /// v1.0 sentinel index name.
        index_name: &'static str,
        /// Query vector components.
        query: &'static [f32],
        /// Requested top-k result count.
        k: usize,
        /// Nullable probe-count override.
        n_probe: Option<u32>,
        /// Optional node-filter IDs. Empty means NULL.
        filter_nodes: &'static [u64],
    },
    /// `vector.ivf_stats`.
    IvfStats {
        /// v1.0 sentinel index name.
        index_name: &'static str,
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
            Self::CreateIndex { .. } => &["vector", "create_index"],
            Self::DropIndex { .. } => &["vector", "drop_index"],
            Self::ListIndexes => &["vector", "list_indexes"],
            Self::IvfBulkUpsert { .. } => &["vector", "ivf_bulk_upsert"],
            Self::IvfBulkDelete { .. } => &["vector", "ivf_bulk_delete"],
            Self::IvfSearch { .. } => &["vector", "ivf_search"],
            Self::IvfStats { .. } => &["vector", "ivf_stats"],
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
                "CALL vector.search({}, {}, {k}, {}, {}, NULL)",
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
            Self::CreateIndex { name, kind, config } => format!(
                "CALL vector.create_index({}, {}, {config})",
                quoted(name),
                quoted(kind)
            ),
            Self::DropIndex { name } => format!("CALL vector.drop_index({})", quoted(name)),
            Self::ListIndexes => "CALL vector.list_indexes()".to_string(),
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
            Self::IvfSearch {
                index_name,
                query,
                k,
                n_probe,
                filter_nodes,
            } => format!(
                "CALL vector.ivf_search({}, {}, {k}, {}, {})",
                quoted(index_name),
                f32_list(query),
                nullable_u32(*n_probe),
                node_filter(filter_nodes)
            ),
            Self::IvfStats { index_name } => {
                format!("CALL vector.ivf_stats({})", quoted(index_name))
            }
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
                name: "create_index_hnsw",
                category: VectorPackCorpusCategory::CreateIndex,
                invocation: VectorPackInvocation::CreateIndex {
                    name: "episodes_hnsw",
                    kind: "hnsw",
                    config: "{dim: 4, metric: 'cosine'}",
                },
            },
            VectorPackCorpusEntry {
                name: "drop_index_hnsw",
                category: VectorPackCorpusCategory::DropIndex,
                invocation: VectorPackInvocation::DropIndex {
                    name: "episodes_hnsw",
                },
            },
            VectorPackCorpusEntry {
                name: "list_indexes",
                category: VectorPackCorpusCategory::ListIndexes,
                invocation: VectorPackInvocation::ListIndexes,
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

    /// Construct the B4 seed corpus, preserving B3 entries in order.
    #[must_use]
    pub fn b4_seed() -> Self {
        let mut corpus = Self::b3_seed();
        corpus.entries.extend([
            VectorPackCorpusEntry {
                name: "ivf_search_default",
                category: VectorPackCorpusCategory::IvfSearch,
                invocation: VectorPackInvocation::IvfSearch {
                    index_name: "default",
                    query: &[1.0, 0.0, 0.0, 0.0],
                    k: 10,
                    n_probe: None,
                    filter_nodes: &[],
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_search_n_probe_override",
                category: VectorPackCorpusCategory::IvfSearch,
                invocation: VectorPackInvocation::IvfSearch {
                    index_name: "default",
                    query: &[0.0, 1.0, 0.0, 0.0],
                    k: 5,
                    n_probe: Some(2),
                    filter_nodes: &[],
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_search_filtered",
                category: VectorPackCorpusCategory::IvfSearch,
                invocation: VectorPackInvocation::IvfSearch {
                    index_name: "default",
                    query: &[0.0, 0.0, 1.0, 0.0],
                    k: 3,
                    n_probe: None,
                    filter_nodes: &[42, 43],
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_stats_trained",
                category: VectorPackCorpusCategory::IvfStats,
                invocation: VectorPackInvocation::IvfStats {
                    index_name: "default",
                },
            },
            VectorPackCorpusEntry {
                name: "ivf_stats_deferred",
                category: VectorPackCorpusCategory::IvfStats,
                invocation: VectorPackInvocation::IvfStats {
                    index_name: "default",
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

fn nullable_u32(value: Option<u32>) -> String {
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
