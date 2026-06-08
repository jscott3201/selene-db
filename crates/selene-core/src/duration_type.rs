//! Duration type qualifiers and duration field-family helpers.

/// ISO temporal duration type qualifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DurationTypeQualifier {
    /// `YEAR TO MONTH`.
    YearToMonth,
    /// `DAY TO SECOND`.
    DayToSecond,
}

/// Field family carried by a concrete duration value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DurationValueFamily {
    /// All fields are zero.
    Zero,
    /// Only year/month fields are non-zero.
    YearMonth,
    /// Only day/time fields are non-zero.
    DayTime,
}

impl DurationTypeQualifier {
    /// Canonical GQL spelling for this qualifier.
    #[must_use]
    pub const fn gql_name(self) -> &'static str {
        match self {
            Self::YearToMonth => "YEAR TO MONTH",
            Self::DayToSecond => "DAY TO SECOND",
        }
    }

    /// Return true when `value` conforms to this qualified duration type.
    #[must_use]
    pub fn matches_span(self, value: &jiff::Span) -> bool {
        matches!(
            (self, duration_value_family(value)),
            (_, Some(DurationValueFamily::Zero))
                | (Self::YearToMonth, Some(DurationValueFamily::YearMonth))
                | (Self::DayToSecond, Some(DurationValueFamily::DayTime))
        )
    }
}

/// Return the duration field family, or `None` when year/month and day/time fields
/// are mixed in one span.
#[must_use]
pub fn duration_value_family(value: &jiff::Span) -> Option<DurationValueFamily> {
    let has_year_month = value.get_years() != 0 || value.get_months() != 0;
    let has_day_time = value.get_weeks() != 0
        || value.get_days() != 0
        || value.get_hours() != 0
        || value.get_minutes() != 0
        || value.get_seconds() != 0
        || value.get_milliseconds() != 0
        || value.get_microseconds() != 0
        || value.get_nanoseconds() != 0;
    match (has_year_month, has_day_time) {
        (false, false) => Some(DurationValueFamily::Zero),
        (true, false) => Some(DurationValueFamily::YearMonth),
        (false, true) => Some(DurationValueFamily::DayTime),
        (true, true) => None,
    }
}
