//! Typed absolute logical catalog paths.

use std::fmt;

use selene_catalog::{CatalogName, IdentifierForm};

use crate::{Error, Result};

/// One validated catalog path segment.
///
/// Segments are logical names, not filesystem components. Equality and
/// ordering use the catalog's case-sensitive NFC canonical form.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PathSegment(pub(crate) CatalogName);

impl PathSegment {
    /// Validate a regular identifier segment.
    ///
    /// # Errors
    ///
    /// Returns an invalid-name error when `value` is outside the selected
    /// catalog identifier profile.
    pub fn regular(value: impl Into<String>) -> Result<Self> {
        CatalogName::regular(value)
            .map(Self)
            .map_err(Error::from_catalog_name)
    }

    /// Validate a decoded delimited identifier segment.
    ///
    /// # Errors
    ///
    /// Returns an invalid-name error for an empty name or private-use scalar.
    pub fn delimited(value: impl Into<String>) -> Result<Self> {
        CatalogName::delimited(value)
            .map(Self)
            .map_err(Error::from_catalog_name)
    }

    /// Return the decoded source spelling.
    #[must_use]
    pub fn display(&self) -> &str {
        self.0.display()
    }

    /// Return the NFC comparison spelling.
    #[must_use]
    pub fn canonical(&self) -> &str {
        self.0.canonical()
    }

    fn render(&self) -> String {
        match self.0.form() {
            Some(IdentifierForm::Regular) => self.display().to_owned(),
            Some(IdentifierForm::Delimited) => {
                format!("`{}`", self.display().replace('`', "``"))
            }
            None => unreachable!("public path segments cannot be synthetic"),
        }
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

macro_rules! logical_path {
    ($name:ident, $doc:literal, $($field:ident),+) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name {
            $(pub(crate) $field: PathSegment),+
        }

        impl $name {
            /// Construct a path from independently validated segments.
            #[must_use]
            pub const fn new($($field: PathSegment),+) -> Self {
                Self { $($field),+ }
            }

            /// Construct a path whose segments are regular identifiers.
            ///
            /// # Errors
            ///
            /// Returns an invalid-name error when any segment is rejected.
            pub fn regular($($field: impl Into<String>),+) -> Result<Self> {
                Ok(Self::new($(PathSegment::regular($field)?),+))
            }

            /// Return the catalog segment.
            #[must_use]
            pub const fn catalog(&self) -> &PathSegment {
                &self.catalog
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                $(write!(formatter, "/{}", self.$field.render())?;)+
                Ok(())
            }
        }
    };
}

logical_path!(
    CatalogPath,
    "Absolute path to the one catalog root.",
    catalog
);
logical_path!(
    SchemaPath,
    "Absolute path to a root-owned schema.",
    catalog,
    schema
);
logical_path!(
    ObjectPath,
    "Absolute path to a graph or graph-type object.",
    catalog,
    schema,
    object
);

impl SchemaPath {
    /// Return the schema segment.
    #[must_use]
    pub const fn schema(&self) -> &PathSegment {
        &self.schema
    }
}

impl ObjectPath {
    /// Return the schema segment.
    #[must_use]
    pub const fn schema(&self) -> &PathSegment {
        &self.schema
    }

    /// Return the object segment.
    #[must_use]
    pub const fn object(&self) -> &PathSegment {
        &self.object
    }

    pub(crate) fn schema_path(&self) -> SchemaPath {
        SchemaPath::new(self.catalog.clone(), self.schema.clone())
    }
}
