//! Shared temporal string parsing for GQL literals and casts.

/// Parsed `DATETIME` text, which may be local or zoned depending on its image.
#[derive(Debug)]
pub(crate) enum ParsedDateTime {
    /// Zoned datetime.
    Zoned(jiff::Zoned),
    /// Local datetime.
    Local(jiff::civil::DateTime),
}

/// Parsed `TIME` text, which may be local or zoned depending on its image.
#[derive(Debug)]
pub(crate) enum ParsedTime {
    /// Zoned time stored in a `Zoned` with an internal anchor date.
    Zoned(jiff::Zoned),
    /// Local time.
    Local(jiff::civil::Time),
}

/// Parse `DATE` text.
pub(crate) fn parse_date(text: &str) -> Result<jiff::civil::Date, String> {
    text.parse::<jiff::civil::Date>()
        .map_err(|error| temporal_error("DATE", error))
}

/// Parse `LOCAL DATETIME` text.
pub(crate) fn parse_local_datetime(text: &str) -> Result<jiff::civil::DateTime, String> {
    let pieces = parse_datetime_pieces(text, "LOCAL DATETIME")?;
    reject_datetime_zone(&pieces, "LOCAL DATETIME")?;
    let time = pieces
        .time()
        .ok_or_else(|| "LOCAL DATETIME literal requires a time".to_owned())?;
    Ok(pieces.date().to_datetime(time))
}

/// Parse `ZONED DATETIME` text.
pub(crate) fn parse_zoned_datetime(text: &str) -> Result<jiff::Zoned, String> {
    parse_zoned_datetime_text(text, "ZONED DATETIME")
}

/// Parse bare `DATETIME` / `TIMESTAMP` text.
pub(crate) fn parse_datetime(text: &str) -> Result<ParsedDateTime, String> {
    let pieces = parse_datetime_pieces(text, "DATETIME")?;
    if has_datetime_zone(&pieces, "DATETIME")? {
        parse_zoned_datetime_text(text, "DATETIME").map(ParsedDateTime::Zoned)
    } else {
        let time = pieces
            .time()
            .ok_or_else(|| "DATETIME literal requires a time".to_owned())?;
        Ok(ParsedDateTime::Local(pieces.date().to_datetime(time)))
    }
}

/// Parse `LOCAL TIME` text.
pub(crate) fn parse_local_time(text: &str) -> Result<jiff::civil::Time, String> {
    if time_has_zone_designator(text) {
        return Err("LOCAL TIME literal must not include a time zone displacement".to_owned());
    }
    text.parse::<jiff::civil::Time>()
        .map_err(|error| temporal_error("LOCAL TIME", error))
}

/// Parse `ZONED TIME` text.
pub(crate) fn parse_zoned_time(text: &str) -> Result<jiff::Zoned, String> {
    parse_zoned_time_text(text)
}

/// Parse bare `TIME` text.
pub(crate) fn parse_time(text: &str) -> Result<ParsedTime, String> {
    if time_has_zone_designator(text) {
        parse_zoned_time_text(text).map(ParsedTime::Zoned)
    } else {
        text.parse::<jiff::civil::Time>()
            .map(ParsedTime::Local)
            .map_err(|error| temporal_error("TIME", error))
    }
}

/// Parse `DURATION` text.
pub(crate) fn parse_duration(text: &str) -> Result<jiff::Span, String> {
    text.parse::<jiff::Span>()
        .map_err(|error| temporal_error("DURATION", error))
}

fn parse_zoned_datetime_text(text: &str, kind: &'static str) -> Result<jiff::Zoned, String> {
    let pieces = parse_datetime_pieces(text, kind)?;
    let time = pieces
        .time()
        .ok_or_else(|| format!("{kind} literal requires a time"))?;
    let zone = pieces
        .to_time_zone()
        .map_err(|error| temporal_error(kind, error))?
        .or_else(|| pieces.to_numeric_offset().map(jiff::tz::TimeZone::fixed))
        .ok_or_else(|| format!("{kind} literal requires a time zone displacement"))?;
    pieces
        .date()
        .to_datetime(time)
        .to_zoned(zone)
        .map_err(|error| temporal_error(kind, error))
}

fn parse_zoned_time_text(text: &str) -> Result<jiff::Zoned, String> {
    if !time_has_zone_designator(text) {
        return Err("ZONED TIME literal requires a time zone displacement".to_owned());
    }
    let anchored = format!("1970-01-01T{text}");
    parse_zoned_datetime_text(&anchored, "ZONED TIME")
}

fn parse_datetime_pieces<'a>(
    text: &'a str,
    kind: &'static str,
) -> Result<jiff::fmt::temporal::Pieces<'a>, String> {
    jiff::fmt::temporal::DateTimeParser::new()
        .parse_pieces(text)
        .map_err(|error| temporal_error(kind, error))
}

fn reject_datetime_zone(
    pieces: &jiff::fmt::temporal::Pieces<'_>,
    kind: &'static str,
) -> Result<(), String> {
    if has_datetime_zone(pieces, kind)? {
        return Err(format!(
            "{kind} literal must not include a time zone displacement"
        ));
    }
    Ok(())
}

fn has_datetime_zone(
    pieces: &jiff::fmt::temporal::Pieces<'_>,
    kind: &'static str,
) -> Result<bool, String> {
    Ok(pieces.to_numeric_offset().is_some()
        || pieces
            .to_time_zone()
            .map_err(|error| temporal_error(kind, error))?
            .is_some())
}

fn time_has_zone_designator(text: &str) -> bool {
    text.ends_with(['Z', 'z']) || text.contains('[') || text.bytes().any(|b| b == b'+' || b == b'-')
}

fn temporal_error(kind: &'static str, error: impl std::fmt::Display) -> String {
    format!("invalid {kind} literal: {error}")
}
