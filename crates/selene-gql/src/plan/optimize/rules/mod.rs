//! Built-in optimizer rules.

mod and_splitting;
mod constant_folding;
mod expand_filter_pushdown;
mod filter_pushdown;
mod top_k;

pub use and_splitting::AndSplitting;
pub use constant_folding::ConstantFolding;
pub use expand_filter_pushdown::ExpandFilterPushdown;
pub use filter_pushdown::FilterPushdown;
pub use top_k::TopK;
