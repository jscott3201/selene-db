//! Facade request values, immutable context, and top-level outcome.

use std::sync::Arc;

use crate::{Error, ExecutionOutcome, RequestParams, Result};

/// One source statement and its request-scoped parameters.
#[derive(Clone, Debug)]
pub struct Request {
    source: String,
    parameters: RequestParams,
}

impl Request {
    /// Construct a request without request-scoped parameters.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            parameters: RequestParams::new(),
        }
    }

    /// Construct a request from a validated parameter dictionary.
    #[must_use]
    pub fn with_params(source: impl Into<String>, parameters: RequestParams) -> Self {
        Self {
            source: source.into(),
            parameters,
        }
    }

    /// Borrow the GQL source statement.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Borrow the request-scoped parameter dictionary.
    #[must_use]
    pub const fn parameters(&self) -> &RequestParams {
        &self.parameters
    }

    /// Mutably borrow the validated request dictionary.
    pub const fn parameters_mut(&mut self) -> &mut RequestParams {
        &mut self.parameters
    }
}

/// Immutable wall-clock instant captured once when a request starts.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestTimestamp(jiff::Timestamp);

impl RequestTimestamp {
    pub(crate) fn capture() -> Self {
        Self(jiff::Timestamp::now())
    }

    #[cfg(test)]
    pub(crate) fn from_parts(seconds: i64, nanoseconds: i32) -> Self {
        Self(jiff::Timestamp::new(seconds, nanoseconds).expect("test timestamp is in range"))
    }

    /// Return whole seconds from the Unix epoch.
    #[must_use]
    pub fn unix_seconds(self) -> i64 {
        self.0.as_second()
    }

    /// Return the signed fractional nanosecond component.
    #[must_use]
    pub fn subsec_nanoseconds(self) -> i32 {
        self.0.subsec_nanosecond()
    }

    pub(crate) const fn lower(self) -> jiff::Timestamp {
        self.0
    }
}

/// Immutable parameter and temporal snapshot associated with one execution.
///
/// The context contains no graph allocation, lock, transaction, physical row,
/// binding-table registry, or execution stack. `TableRef` parameters are
/// rejected before execution until M03-PR03 defines their request registry.
#[derive(Debug)]
pub struct RequestContext {
    parameters: RequestParams,
    timestamp: RequestTimestamp,
}

impl RequestContext {
    pub(crate) const fn new(parameters: RequestParams, timestamp: RequestTimestamp) -> Self {
        Self {
            parameters,
            timestamp,
        }
    }

    /// Borrow the merged session/request parameter snapshot.
    #[must_use]
    pub const fn parameters(&self) -> &RequestParams {
        &self.parameters
    }

    /// Return the instant captured when this request began.
    #[must_use]
    pub const fn timestamp(&self) -> RequestTimestamp {
        self.timestamp
    }

    pub(crate) fn lower_input(
        &self,
        time_zone_seconds: i32,
    ) -> Result<selene_gql::RequestExecutionInput> {
        let offset = jiff::tz::Offset::from_seconds(time_zone_seconds)
            .map_err(Error::invalid_request_time_zone)?;
        Ok(selene_gql::RequestExecutionInput::new(
            self.parameters.to_lower(),
            self.timestamp.lower(),
            jiff::tz::TimeZone::fixed(offset),
        ))
    }
}

/// Uniform result of request validation, compilation, dispatch, and execution.
#[derive(Debug)]
#[non_exhaustive]
pub enum RequestOutcome {
    /// Request execution completed with the existing statement summary.
    Succeeded {
        /// Context that was active for the request.
        context: Arc<RequestContext>,
        /// Existing statement-level summary.
        outcome: ExecutionOutcome,
    },
    /// Request validation, compilation, dispatch, or execution failed.
    Failed {
        /// Context that was active, or was refused because another was active.
        context: Arc<RequestContext>,
        /// Facade diagnostic with its original source chain and GQLSTATUS.
        error: Error,
    },
}

impl RequestOutcome {
    /// Borrow the request context retained by either outcome variant.
    #[must_use]
    pub const fn context(&self) -> &Arc<RequestContext> {
        match self {
            Self::Succeeded { context, .. } | Self::Failed { context, .. } => context,
        }
    }

    /// Borrow the successful statement summary, when present.
    #[must_use]
    pub const fn execution(&self) -> Option<&ExecutionOutcome> {
        match self {
            Self::Succeeded { outcome, .. } => Some(outcome),
            Self::Failed { .. } => None,
        }
    }

    /// Borrow the request failure, when present.
    #[must_use]
    pub const fn error(&self) -> Option<&Error> {
        match self {
            Self::Failed { error, .. } => Some(error),
            Self::Succeeded { .. } => None,
        }
    }

    /// Convert to the compatibility result returned by [`Session::execute`](crate::Session::execute).
    pub fn into_result(self) -> Result<ExecutionOutcome> {
        match self {
            Self::Succeeded { outcome, .. } => Ok(outcome),
            Self::Failed { error, .. } => Err(error),
        }
    }
}
