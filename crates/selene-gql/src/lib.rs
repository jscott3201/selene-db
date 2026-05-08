//! ISO/IEC 39075:2024 GQL parser, AST, and Flagger for selene-db.
//!
//! See `_spec/07-iso-gql-parser-and-flagger.md` for the design contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod ast;
pub mod error;
pub mod parser;

pub use crate::ast::{
    expr::{Literal, ValueExpr},
    span::SourceSpan,
    statement::{ReturnItem, ReturnStatement, Statement},
};
pub use crate::error::{GqlStatus, ParserError};
pub use crate::parser::parse;
