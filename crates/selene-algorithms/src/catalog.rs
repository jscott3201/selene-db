//! Named projection cache with generation-based staleness detection.
//!
//! [`ProjectionCatalog`] lets embedders amortize CSR build cost across
//! repeated algorithm runs against the same logical subgraph. Each entry pairs
//! a built [`GraphProjection`] with the [`ProjectionConfig`] that produced it,
//! so [`ProjectionCatalog::ensure_fresh`] can rebuild from the stored recipe
//! when the underlying graph generation advances.
//!
//! ## Catalog projections are always unscoped
//!
//! The stored [`ProjectionConfig`] is the *complete* rebuild recipe, and it
//! carries no scope bitmap. Accepting a scope at registration would silently
//! widen the projection on the first stale rebuild, so the catalog refuses
//! the parameter entirely: [`ProjectionCatalog::project`] always builds
//! unscoped (spec 16 §3 E06). Callers needing a scoped, point-in-time view
//! build one directly via [`GraphProjection::build`] and manage its lifetime
//! themselves.
//!
//! ## Concurrency
//!
//! The catalog is `Send + Sync`. Reads (`get`, `len`, `is_empty`, `contains`,
//! `names`) acquire a `parking_lot::RwLock` read guard; mutations (`project`,
//! `drop_projection`, and the rebuild branch of `ensure_fresh`) acquire the
//! write guard. The [`ProjectionRef`] returned by `get` holds the read guard
//! for its lifetime — callers MUST drop the ref before invoking any mutation,
//! or the mutation will block on the read lock (per spec 16 §3 E07).

use std::collections::HashMap;

use parking_lot::{RwLock, RwLockReadGuard};
use selene_graph::SeleneGraph;

use crate::error::AlgorithmsError;
use crate::projection::{GraphProjection, ProjectionConfig};

/// A cached projection paired with the config that produced it.
///
/// Storing the config alongside the projection lets `ensure_fresh` rebuild
/// from the original recipe without forcing callers to re-pass it.
#[derive(Debug)]
struct CatalogEntry {
    projection: GraphProjection,
    config: ProjectionConfig,
}

/// Named cache of [`GraphProjection`]s with generation-based staleness
/// detection.
///
/// Use [`ProjectionCatalog::project`] to register a projection under a name,
/// [`ProjectionCatalog::ensure_fresh`] to refresh it against the current
/// snapshot generation, and [`ProjectionCatalog::get`] for read-locked access.
#[derive(Debug)]
pub struct ProjectionCatalog {
    entries: RwLock<HashMap<String, CatalogEntry>>,
}

impl Default for ProjectionCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectionCatalog {
    /// Construct an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Build (or rebuild) a named projection from `config`.
    ///
    /// Returns `(node_count, edge_count)`. Overwrites any existing projection
    /// of the same name — equivalent to `drop_projection(&config.name)` then
    /// build, but in one atomic write-lock acquisition.
    ///
    /// The projection is always **unscoped**: `config` is the complete recipe
    /// that [`Self::ensure_fresh`] rebuilds from, and it cannot retain a scope
    /// bitmap across rebuilds (spec 16 §3 E06). Scoped views go through
    /// [`GraphProjection::build`] directly, outside the catalog.
    pub fn project(
        &self,
        snapshot: &SeleneGraph,
        config: &ProjectionConfig,
    ) -> Result<(usize, usize), AlgorithmsError> {
        let projection = GraphProjection::build(snapshot, config, None)?;
        let node_count = projection.node_count();
        let edge_count = projection.edge_count();
        self.entries.write().insert(
            config.name.clone(),
            CatalogEntry {
                projection,
                config: config.clone(),
            },
        );
        Ok((node_count, edge_count))
    }

    /// Ensure the named projection exists and is fresh against `snapshot`.
    ///
    /// - If absent: returns [`AlgorithmsError::NoSuchProjection`].
    /// - If present and generation matches `snapshot.meta.generation`: no-op.
    /// - If present and generation differs: rebuild from the stored config.
    ///   Because catalog projections are unscoped by construction (spec 16
    ///   §3 E06), the rebuild reproduces exactly what [`Self::project`]
    ///   registered, evaluated against the fresh snapshot.
    pub fn ensure_fresh(&self, snapshot: &SeleneGraph, name: &str) -> Result<(), AlgorithmsError> {
        let current_gen = snapshot.meta.generation;

        // Phase 1: fast-path read-lock check.
        {
            let guard = self.entries.read();
            match guard.get(name) {
                None => {
                    return Err(AlgorithmsError::NoSuchProjection {
                        name: name.to_string(),
                    });
                }
                Some(entry) if entry.projection.generation() == current_gen => {
                    return Ok(());
                }
                Some(_) => {
                    // stale; fall through to rebuild under the write lock.
                }
            }
        }

        // Phase 2: write-lock rebuild path.
        let mut guard = self.entries.write();
        // Re-check existence + freshness under write lock: another writer
        // may have inserted (or refreshed) the entry between our Phase 1
        // read drop and the Phase 2 write acquisition.
        let entry = guard
            .get(name)
            .ok_or_else(|| AlgorithmsError::NoSuchProjection {
                name: name.to_string(),
            })?;
        if entry.projection.generation() == current_gen {
            return Ok(());
        }
        let config = entry.config.clone();
        let projection = GraphProjection::build(snapshot, &config, None)?;
        guard.insert(name.to_string(), CatalogEntry { projection, config });
        Ok(())
    }

    /// Read-locked access to a named projection. Returns `None` when absent.
    ///
    /// The returned [`ProjectionRef`] holds the catalog's read guard for its
    /// lifetime; callers MUST drop it before invoking any mutation
    /// (`project`, `drop_projection`, or a rebuilding `ensure_fresh`).
    /// Mutations block on the write lock and would deadlock otherwise.
    pub fn get<'a>(&'a self, name: &str) -> Option<ProjectionRef<'a>> {
        let guard = self.entries.read();
        if guard.contains_key(name) {
            Some(ProjectionRef {
                guard,
                name: name.to_string(),
            })
        } else {
            None
        }
    }

    /// Remove the named projection. Returns `true` if it existed.
    pub fn drop_projection(&self, name: &str) -> bool {
        self.entries.write().remove(name).is_some()
    }

    /// Number of registered projections.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// True when no projections are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// True when the named projection is registered.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.entries.read().contains_key(name)
    }

    /// Snapshot of all registered projection names. Returns an owned `Vec`
    /// so the read lock releases before the caller iterates.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.entries.read().keys().cloned().collect()
    }
}

/// Read guard providing access to a single projection in a
/// [`ProjectionCatalog`].
///
/// Why: `ProjectionRef` holds the catalog's read guard pinning this entry.
/// Cloning would break the same-lock invariant that lets us assume the entry
/// stays alive for the ref's lifetime, so `ProjectionRef` deliberately does
/// not implement [`Clone`].
pub struct ProjectionRef<'a> {
    guard: RwLockReadGuard<'a, HashMap<String, CatalogEntry>>,
    name: String,
}

impl std::fmt::Debug for ProjectionRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionRef")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl ProjectionRef<'_> {
    /// Borrow the underlying [`GraphProjection`] from the pinned entry.
    #[must_use]
    pub fn projection(&self) -> &GraphProjection {
        &self
            .guard
            .get(&self.name)
            .expect(
                "ProjectionRef holds the same read guard that pinned this entry at hand-out time; \
                 drop_projection and project require the write lock and would block on this read.",
            )
            .projection
    }
}

impl std::ops::Deref for ProjectionRef<'_> {
    type Target = GraphProjection;
    fn deref(&self) -> &Self::Target {
        self.projection()
    }
}
