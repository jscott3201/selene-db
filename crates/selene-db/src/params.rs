//! Typed facade request and session parameters.

use std::collections::BTreeMap;

use selene_core::DbString;

use crate::{Error, GqlType, Result, Value};

/// One explicitly declared GQL parameter value.
///
/// Construction uses the lower runtime's structural type matcher. The same
/// matcher checks inline source declarations during request preflight.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralParameter {
    declared_type: GqlType,
    value: Value,
}

impl GeneralParameter {
    /// Validate and construct a typed parameter.
    ///
    /// # Errors
    ///
    /// Returns `22G03` when `value` does not satisfy `declared_type`.
    pub fn new(declared_type: GqlType, value: Value) -> Result<Self> {
        selene_gql::validate_parameter_value(&value, &declared_type).map_err(Error::from_engine)?;
        Ok(Self {
            declared_type,
            value,
        })
    }

    /// Borrow the parameter's explicit declaration.
    #[must_use]
    pub const fn declared_type(&self) -> &GqlType {
        &self.declared_type
    }

    /// Borrow the parameter value.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn to_lower(&self) -> selene_gql::RequestParameter {
        selene_gql::RequestParameter::new(self.declared_type.clone(), self.value.clone())
    }
}

/// Deterministic exact-name request parameter dictionary.
///
/// Names omit `$`, are case-sensitive, and follow the parser's Unicode
/// parameter rule. Insertion rejects an existing exact name rather than
/// replacing it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestParams {
    entries: BTreeMap<DbString, GeneralParameter>,
}

impl RequestParams {
    /// Construct an empty dictionary.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Insert a parameter after validating its decoded name.
    ///
    /// # Errors
    ///
    /// Returns an invalid-name diagnostic for spellings the GQL parser would
    /// reject after `$`, or a duplicate diagnostic when the exact name exists.
    pub fn insert(&mut self, name: &str, parameter: GeneralParameter) -> Result<()> {
        let name = validated_parameter_name(name)?;
        if self.entries.contains_key(&name) {
            return Err(Error::duplicate_parameter(name.as_str()));
        }
        self.entries.insert(name, parameter);
        Ok(())
    }

    /// Borrow one exact-name parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&GeneralParameter> {
        self.entries.get(name)
    }

    /// Return the number of request parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether this dictionary is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate in exact-name lexical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &GeneralParameter)> {
        self.entries
            .iter()
            .map(|(name, parameter)| (name.as_str(), parameter))
    }

    pub(crate) fn overlay(session: &BTreeMap<DbString, GeneralParameter>, request: &Self) -> Self {
        let mut entries = session.clone();
        entries.extend(request.entries.clone());
        Self { entries }
    }

    pub(crate) fn to_lower(&self) -> BTreeMap<DbString, selene_gql::RequestParameter> {
        self.entries
            .iter()
            .map(|(name, parameter)| (name.clone(), parameter.to_lower()))
            .collect()
    }
}

pub(crate) fn validated_parameter_name(name: &str) -> Result<DbString> {
    if !selene_gql::is_parameter_name(name) {
        return Err(Error::invalid_parameter_name(name));
    }
    selene_core::db_string(name)
        .map_err(|source| Error::invalid_parameter_name_source(name, source))
}
