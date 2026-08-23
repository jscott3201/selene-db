//! Versioned catalog identifier validation and canonical comparison names.

use std::{cmp::Ordering, hash::Hash};

use serde::{Deserialize, Deserializer, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::{CatalogError, CatalogResult};

/// Unicode data version used for catalog identifiers and NFC normalization.
pub const CATALOG_UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);

const _: () = {
    assert!(unicode_ident::UNICODE_VERSION.0 == CATALOG_UNICODE_VERSION.0);
    assert!(unicode_ident::UNICODE_VERSION.1 == CATALOG_UNICODE_VERSION.1);
    assert!(unicode_ident::UNICODE_VERSION.2 == CATALOG_UNICODE_VERSION.2);
    assert!(unicode_normalization::UNICODE_VERSION.0 == CATALOG_UNICODE_VERSION.0);
    assert!(unicode_normalization::UNICODE_VERSION.1 == CATALOG_UNICODE_VERSION.1);
    assert!(unicode_normalization::UNICODE_VERSION.2 == CATALOG_UNICODE_VERSION.2);
};

/// Source form used to validate a user catalog identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierForm {
    /// UAX #31 R1-2 profile of R1-1: XID_Start/XID_Continue plus U+005F LOW LINE at start and continuation.
    Regular,
    /// Decoded delimited spelling, unconstrained by XID properties.
    Delimited,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NameForm {
    Regular,
    Delimited,
    SyntheticRoot,
}

/// Immutable catalog name retaining display spelling and precomputed NFC identity.
///
/// Equality, hashing, and ordering use only [`CatalogName::canonical`]. Descriptor
/// equality separately compares display spelling and source form as metadata.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogName {
    display: String,
    canonical: String,
    form: NameForm,
}

impl CatalogName {
    /// Validate a decoded regular identifier.
    ///
    /// # Errors
    ///
    /// Rejects empty names, private-use scalars, and characters outside the
    /// selected XID profile with LOW LINE tailoring.
    pub fn regular(display: impl Into<String>) -> CatalogResult<Self> {
        Self::user(display.into(), NameForm::Regular)
    }

    /// Validate a decoded delimited identifier.
    ///
    /// # Errors
    ///
    /// Rejects empty names and private-use scalars. XID properties do not
    /// constrain delimited spellings.
    pub fn delimited(display: impl Into<String>) -> CatalogResult<Self> {
        Self::user(display.into(), NameForm::Delimited)
    }

    fn user(display: String, form: NameForm) -> CatalogResult<Self> {
        if display.is_empty() {
            return Err(CatalogError::EmptyIdentifier);
        }
        reject_private_use(&display)?;
        if form == NameForm::Regular {
            validate_regular(&display)?;
        }
        let canonical = display.nfc().collect();
        Ok(Self {
            display,
            canonical,
            form,
        })
    }

    pub(crate) fn synthetic_root() -> Self {
        Self {
            display: String::new(),
            canonical: String::new(),
            form: NameForm::SyntheticRoot,
        }
    }

    /// Return the decoded source spelling retained for diagnostics.
    #[must_use]
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Return the NFC comparison spelling.
    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// Return the user identifier form, or `None` for the synthetic root.
    #[must_use]
    pub const fn form(&self) -> Option<IdentifierForm> {
        match self.form {
            NameForm::Regular => Some(IdentifierForm::Regular),
            NameForm::Delimited => Some(IdentifierForm::Delimited),
            NameForm::SyntheticRoot => None,
        }
    }

    /// Return whether this is the encapsulated zero-length root name.
    #[must_use]
    pub const fn is_synthetic_root(&self) -> bool {
        matches!(self.form, NameForm::SyntheticRoot)
    }

    pub(crate) fn metadata_eq(&self, other: &Self) -> bool {
        self.display == other.display
            && self.canonical == other.canonical
            && self.form == other.form
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.display.capacity() + self.canonical.capacity()
    }
}

impl<'de> Deserialize<'de> for CatalogName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireName {
            display: String,
            canonical: String,
            form: NameForm,
        }

        let wire = WireName::deserialize(deserializer)?;
        let name = match wire.form {
            NameForm::Regular => Self::regular(wire.display),
            NameForm::Delimited => Self::delimited(wire.display),
            NameForm::SyntheticRoot if wire.display.is_empty() => Ok(Self::synthetic_root()),
            NameForm::SyntheticRoot => Err(CatalogError::InvalidSerializedName),
        }
        .map_err(serde::de::Error::custom)?;
        if name.canonical != wire.canonical {
            return Err(serde::de::Error::custom(
                CatalogError::InvalidSerializedName,
            ));
        }
        Ok(name)
    }
}

impl PartialEq for CatalogName {
    fn eq(&self, other: &Self) -> bool {
        self.canonical == other.canonical
    }
}

impl Eq for CatalogName {}

impl PartialOrd for CatalogName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CatalogName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical.chars().cmp(other.canonical.chars())
    }
}

impl Hash for CatalogName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.canonical.hash(state);
    }
}

fn validate_regular(display: &str) -> CatalogResult<()> {
    let mut chars = display.chars();
    let first = chars.next().expect("nonempty checked by caller");
    if first != '_' && !unicode_ident::is_xid_start(first) {
        return Err(CatalogError::InvalidRegularIdentifierStart { character: first });
    }
    for (index, character) in chars.enumerate() {
        if character != '_' && !unicode_ident::is_xid_continue(character) {
            return Err(CatalogError::InvalidRegularIdentifierContinue {
                index: index + 1,
                character,
            });
        }
    }
    Ok(())
}

fn reject_private_use(display: &str) -> CatalogResult<()> {
    if let Some(character) = display.chars().find(|character| {
        matches!(
            *character as u32,
            0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
        )
    }) {
        return Err(CatalogError::PrivateUseCharacter { character });
    }
    Ok(())
}
