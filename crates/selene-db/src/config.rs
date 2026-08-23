//! Database construction configuration.

/// Storage mode supported by the current facade.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum OpenMode {
    /// Keep all graph state in process memory.
    #[default]
    InMemory,
}

/// Configuration consumed by [`DatabaseBuilder`](crate::DatabaseBuilder).
///
/// M02-PR01 exposes no setters because in-memory operation is the only
/// implemented mode. Future fields are added with the work that consumes them;
/// this type never accepts ignored configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DatabaseConfig {
    open_mode: OpenMode,
}

impl DatabaseConfig {
    /// Return the selected storage mode.
    #[must_use]
    pub const fn open_mode(&self) -> OpenMode {
        self.open_mode
    }
}
