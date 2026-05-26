//! Runtime change-fanout protocol for extension providers.

use selene_core::{Change, ChangeKindSet};

use crate::{ProviderError, ProviderTag};

/// Consumer of committed core graph mutations.
///
/// `ChangeSubscriber` is a sibling to [`crate::IndexProvider`]. Index providers
/// own snapshot sections and extension-originated WAL events; subscribers react
/// to core graph mutations whose semantics they understand.
///
/// ## Re-entrancy contract
///
/// `on_change` MUST NOT initiate a write transaction on the same graph,
/// directly or indirectly. The runtime fan-out boundary detects same-thread
/// re-entry through the same guard used for index providers, catches panics
/// after commit publication, and logs subscriber drift without aborting the
/// already-published graph commit.
pub trait ChangeSubscriber: Send + Sync + 'static {
    /// Stable four-byte tag used for validation and logging.
    fn subscriber_tag(&self) -> ProviderTag;

    /// Declarative filter for changes this subscriber wants to receive.
    ///
    /// The default is empty so a subscriber that forgets to declare its
    /// interests receives no events.
    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY
    }

    /// React to one committed change selected by [`Self::change_kinds`].
    ///
    /// # Errors
    ///
    /// Returns provider-owned errors for subscriber-local failures. Runtime
    /// commits log and continue after these errors; recovery fails with the
    /// propagated error because recovered provider state would otherwise drift.
    fn on_change(&self, change: &Change) -> Result<(), ProviderError>;
}
