//! Mirror corpus for `selene-algorithms-pack` procedure invocations.

/// Stable category tag for algorithms-pack corpus entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlgoPackCorpusCategory {
    /// Projection-catalog procedure coverage.
    Projection,
    /// Algorithm adapter procedure coverage.
    Algorithm,
}

/// Procedure invocation mirrored by the algorithms-pack corpus.
#[derive(Clone, Debug, PartialEq)]
pub enum AlgoPackInvocation {
    /// `algo.projection_build`.
    ProjectionBuild {
        /// Projection name.
        name: &'static str,
        /// Node label filter.
        node_labels: &'static [&'static str],
        /// Edge label filter.
        edge_labels: &'static [&'static str],
        /// Optional edge-weight property.
        weight_property: Option<&'static str>,
    },
    /// `algo.projection_get`.
    ProjectionGet {
        /// Projection name.
        name: &'static str,
    },
    /// `algo.projection_drop`.
    ProjectionDrop {
        /// Projection name.
        name: &'static str,
    },
    /// `algo.projection_list`.
    ProjectionList,
    /// `algo.pagerank`.
    Pagerank {
        /// Projection name.
        projection_name: &'static str,
        /// Nullable damping argument.
        damping: Option<f64>,
        /// Nullable max-iteration argument.
        max_iterations: Option<usize>,
        /// Nullable tolerance argument.
        tolerance: Option<f64>,
    },
}

impl AlgoPackInvocation {
    /// Render a deterministic GQL `CALL` string for snapshot testing.
    #[must_use]
    pub fn render_call(&self) -> String {
        match self {
            Self::ProjectionBuild {
                name,
                node_labels,
                edge_labels,
                weight_property,
            } => format!(
                "CALL algo.projection_build({}, {}, {}, {})",
                quoted(name),
                label_list(node_labels),
                label_list(edge_labels),
                nullable_string(*weight_property)
            ),
            Self::ProjectionGet { name } => {
                format!("CALL algo.projection_get({})", quoted(name))
            }
            Self::ProjectionDrop { name } => {
                format!("CALL algo.projection_drop({})", quoted(name))
            }
            Self::ProjectionList => "CALL algo.projection_list()".to_string(),
            Self::Pagerank {
                projection_name,
                damping,
                max_iterations,
                tolerance,
            } => format!(
                "CALL algo.pagerank({}, {}, {}, {})",
                quoted(projection_name),
                nullable_f64(*damping),
                nullable_usize(*max_iterations),
                nullable_f64(*tolerance)
            ),
        }
    }
}

/// One algorithms-pack corpus entry.
#[derive(Clone, Debug, PartialEq)]
pub struct AlgoPackCorpusEntry {
    /// Stable entry name.
    pub name: &'static str,
    /// Category for coverage accounting.
    pub category: AlgoPackCorpusCategory,
    /// Mirrored invocation.
    pub invocation: AlgoPackInvocation,
}

/// Corpus mirror used by algorithms-pack tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlgoPackCorpus {
    entries: Vec<AlgoPackCorpusEntry>,
}

impl AlgoPackCorpus {
    /// Construct the B1 seed corpus.
    #[must_use]
    pub fn b1_seed() -> Self {
        Self {
            entries: vec![
                AlgoPackCorpusEntry {
                    name: "projection_build_all",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionBuild {
                        name: "p",
                        node_labels: &[],
                        edge_labels: &[],
                        weight_property: None,
                    },
                },
                AlgoPackCorpusEntry {
                    name: "projection_get",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionGet { name: "p" },
                },
                AlgoPackCorpusEntry {
                    name: "projection_drop",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionDrop { name: "p" },
                },
                AlgoPackCorpusEntry {
                    name: "projection_list",
                    category: AlgoPackCorpusCategory::Projection,
                    invocation: AlgoPackInvocation::ProjectionList,
                },
                AlgoPackCorpusEntry {
                    name: "pagerank_defaults",
                    category: AlgoPackCorpusCategory::Algorithm,
                    invocation: AlgoPackInvocation::Pagerank {
                        projection_name: "p",
                        damping: None,
                        max_iterations: None,
                        tolerance: None,
                    },
                },
            ],
        }
    }

    /// Borrow corpus entries in deterministic order.
    #[must_use]
    pub fn entries(&self) -> &[AlgoPackCorpusEntry] {
        &self.entries
    }

    /// Render the corpus to a deterministic snapshot string.
    #[must_use]
    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(|entry| {
                format!(
                    "{} [{:?}] {}\n",
                    entry.name,
                    entry.category,
                    entry.invocation.render_call()
                )
            })
            .collect()
    }
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

fn label_list(labels: &[&str]) -> String {
    if labels.is_empty() {
        return "NULL".to_string();
    }
    let rendered = labels.iter().map(|label| quoted(label)).collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

fn nullable_string(value: Option<&str>) -> String {
    value.map_or_else(|| "NULL".to_string(), quoted)
}

fn nullable_f64(value: Option<f64>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| value.to_string())
}

fn nullable_usize(value: Option<usize>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| value.to_string())
}
