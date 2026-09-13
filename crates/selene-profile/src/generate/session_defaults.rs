//! Typed session-default output generation.

use std::fmt::Write as _;

use crate::{
    ImplementationDefinedDecision, ImplementationDefinedValue, ProfileError, ValidatedProfile,
};

pub(super) fn render(profile: &ValidatedProfile) -> Result<String, ProfileError> {
    let displacement = selected_value(profile, "ID048")?;
    let ImplementationDefinedValue::Identifier {
        value: displacement,
    } = displacement
    else {
        return Err(ProfileError::Invalid(
            "ID048 must select a fixed UTC displacement identifier".to_owned(),
        ));
    };
    let displacement_seconds = parse_fixed_utc_displacement(displacement).ok_or_else(|| {
        ProfileError::Invalid("ID048 must use the fixed displacement shape UTC[+-]HH:MM".to_owned())
    })?;

    let parameter_count = selected_value(profile, "ID049")?;
    let ImplementationDefinedValue::UnsignedInteger {
        value: parameter_count,
    } = parameter_count
    else {
        return Err(ProfileError::Invalid(
            "ID049 must select an unsigned initial parameter count".to_owned(),
        ));
    };
    if *parameter_count != 0 {
        return Err(ProfileError::Invalid(
            "ID049 must select zero until initial parameter values are represented".to_owned(),
        ));
    }

    let session_user = selected_value(profile, "ID061")?;
    let ImplementationDefinedValue::Identifier {
        value: session_user,
    } = session_user
    else {
        return Err(ProfileError::Invalid(
            "ID061 must select a declared type identifier".to_owned(),
        ));
    };
    if session_user != "STRING" {
        return Err(ProfileError::Invalid(
            "ID061 must select STRING for the generated session default".to_owned(),
        ));
    }

    let mut output = super::header(profile);
    output.push_str(
        "use crate::runtime::{FixedTimeZoneDisplacement, SessionDefaults, SessionUserDeclaredType};\n\n",
    );
    writeln!(
        output,
        "pub(crate) const SESSION_DEFAULTS: SessionDefaults = SessionDefaults::new(\n    FixedTimeZoneDisplacement::new({displacement_seconds}),\n    {parameter_count},\n    SessionUserDeclaredType::String,\n);"
    )
    .expect("writing to String cannot fail");
    Ok(output)
}

fn selected_value<'a>(
    profile: &'a ValidatedProfile,
    id: &str,
) -> Result<&'a ImplementationDefinedValue, ProfileError> {
    let record = profile
        .profile()
        .implementation_defined_choices
        .iter()
        .find(|record| record.id.as_str() == id)
        .ok_or_else(|| ProfileError::Invalid(format!("missing generated session default {id}")))?;
    let ImplementationDefinedDecision::Selected { value, .. } = &record.decision else {
        return Err(ProfileError::Invalid(format!(
            "{id} must be selected for generated session defaults"
        )));
    };
    Ok(value)
}

fn parse_fixed_utc_displacement(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.len() != 9
        || &bytes[..3] != b"UTC"
        || !matches!(bytes[3], b'+' | b'-')
        || bytes[6] != b':'
        || !bytes[4..6].iter().all(u8::is_ascii_digit)
        || !bytes[7..9].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let hours = i32::from(bytes[4] - b'0') * 10 + i32::from(bytes[5] - b'0');
    let minutes = i32::from(bytes[7] - b'0') * 10 + i32::from(bytes[8] - b'0');
    if hours > 23 || minutes > 59 {
        return None;
    }
    let seconds = hours * 3_600 + minutes * 60;
    Some(if bytes[3] == b'-' { -seconds } else { seconds })
}
