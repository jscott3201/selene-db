//! Exact public ISO Annex B identifier inventory.
//!
//! This oracle intentionally contains identifiers and category structure only.
//! Profile values, applicability, rationale, evidence, and ownership remain in
//! `spec/gql-profile/profile.json`.

/// Action identifiers.
pub(crate) const IA: &[&str] = &[
    "IA001", "IA002", "IA003", "IA004", "IA005", "IA006", "IA007", "IA010", "IA011", "IA012",
    "IA013", "IA014", "IA015", "IA016", "IA017", "IA019", "IA020", "IA021", "IA023", "IA025",
    "IA026",
];

/// Default identifiers.
pub(crate) const ID: &[&str] = &[
    "ID001", "ID002", "ID003", "ID004", "ID005", "ID006", "ID016", "ID017", "ID022", "ID023",
    "ID028", "ID034", "ID037", "ID048", "ID049", "ID057", "ID058", "ID059", "ID061", "ID062",
    "ID063", "ID064", "ID065", "ID066", "ID067", "ID068", "ID069", "ID070", "ID074", "ID075",
    "ID076", "ID079", "ID085", "ID086", "ID089", "ID090", "ID091", "ID095", "ID096", "ID097",
    "ID098", "ID099",
];

/// Extension identifiers.
pub(crate) const IE: &[&str] = &[
    "IE001", "IE002", "IE003", "IE004", "IE005", "IE006", "IE007", "IE008", "IE009", "IE010",
];

/// Limit identifiers.
pub(crate) const IL: &[&str] = &[
    "IL001", "IL002", "IL003", "IL009", "IL010", "IL011", "IL013", "IL015", "IL018", "IL020",
    "IL023", "IL024",
];

/// Sequencing identifiers.
pub(crate) const IS: &[&str] = &["IS001"];

/// Value identifiers.
pub(crate) const IV: &[&str] = &[
    "IV001", "IV002", "IV003", "IV008", "IV010", "IV011", "IV012", "IV014", "IV015", "IV016",
    "IV023",
];

/// Ways-and-means identifiers.
pub(crate) const IW: &[&str] = &[
    "IW001", "IW002", "IW003", "IW004", "IW005", "IW006", "IW007", "IW010", "IW011", "IW012",
    "IW014", "IW015", "IW016", "IW017", "IW018", "IW019", "IW021", "IW022", "IW023", "IW025",
];

/// Categories in deterministic report order.
pub(crate) const CATEGORIES: &[(&str, &[&str])] = &[
    ("IA", IA),
    ("ID", ID),
    ("IE", IE),
    ("IL", IL),
    ("IS", IS),
    ("IV", IV),
    ("IW", IW),
];

/// Number of exact singleton identifiers.
pub(crate) const TOTAL: usize = 117;
