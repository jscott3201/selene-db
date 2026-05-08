//! Graph-pattern AST shells.

/// Graph pattern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum GraphPattern {}

/// Path pattern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum PathPattern {}
