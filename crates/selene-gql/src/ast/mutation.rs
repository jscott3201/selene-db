//! Mutation, DDL, and transaction statement AST shells.

/// Data-mutation statement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum MutationStatement {}

/// Data-definition statement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum DataDefinitionStatement {}

/// Transaction-control statement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum TransactionControlStatement {}
