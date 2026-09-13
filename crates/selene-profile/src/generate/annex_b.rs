//! Category-sharded Annex B Rust generation.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::{
    DecisionStability, DecisionVisibility, ImplementationDefinedChoiceRecord,
    ImplementationDefinedDecision, ImplementationDefinedValue, ValidatedProfile,
};

use super::header;

const CATEGORIES: &[&str] = &["IA", "ID", "IE", "IL", "IS", "IV", "IW"];

pub(super) fn render_categories(profile: &ValidatedProfile) -> Vec<(PathBuf, String)> {
    CATEGORIES
        .iter()
        .map(|category| {
            (
                PathBuf::from(format!(
                    "crates/selene-profile/src/generated/annex_b_{}.rs",
                    category.to_ascii_lowercase()
                )),
                render_category(profile, category),
            )
        })
        .collect()
}

pub(super) fn render_index(profile: &ValidatedProfile) -> String {
    let mut output = header(profile);
    output.push_str(
        "use crate::runtime::{AnnexBId, AnnexBRecord};\n\n\
         use super::{\n    annex_b_ia::ANNEX_B_IA, annex_b_id::ANNEX_B_ID, annex_b_ie::ANNEX_B_IE, annex_b_il::ANNEX_B_IL,\n    annex_b_is::ANNEX_B_IS, annex_b_iv::ANNEX_B_IV, annex_b_iw::ANNEX_B_IW,\n};\n\n\
         /// Exact category counts in report order.\n\
         pub const ANNEX_B_CATEGORY_COUNTS: &[(&str, usize)] = &[\n",
    );
    for category in CATEGORIES {
        let count = records(profile, category).len();
        writeln!(output, "    ({category:?}, {count}),").expect("String writes cannot fail");
    }
    output.push_str(
        "];\n\n/// Stable lookup vectors mapping every ID to runtime order.\n\
         #[rustfmt::skip]\n\
         pub const ANNEX_B_LOOKUP_TEST_VECTORS: &[(AnnexBId, usize)] = &[\n",
    );
    let mut all = profile
        .profile()
        .implementation_defined_choices
        .iter()
        .collect::<Vec<_>>();
    all.sort_by_key(|record| record.runtime_order);
    for record in all {
        writeln!(
            output,
            "    (AnnexBId::new({:?}), {}),",
            record.id.as_str(),
            record.runtime_order
        )
        .expect("String writes cannot fail");
    }
    output.push_str(
        "];\n\n/// Iterate all Annex B records in category and runtime order.\n\
         pub fn annex_b_records() -> impl Iterator<Item = &'static AnnexBRecord> {\n    \
         ANNEX_B_IA\n        .iter()\n        .chain(ANNEX_B_ID)\n        .chain(ANNEX_B_IE)\n        .chain(ANNEX_B_IL)\n        .chain(ANNEX_B_IS)\n        .chain(ANNEX_B_IV)\n        .chain(ANNEX_B_IW)\n}\n\n\
         /// Look up one exact Annex B singleton identifier.\n\
         #[must_use]\n\
         pub fn annex_b_by_id(id: &str) -> Option<&'static AnnexBRecord> {\n    \
         let category = match id.get(..2) {\n        Some(\"IA\") => ANNEX_B_IA,\n        Some(\"ID\") => ANNEX_B_ID,\n        Some(\"IE\") => ANNEX_B_IE,\n        Some(\"IL\") => ANNEX_B_IL,\n        Some(\"IS\") => ANNEX_B_IS,\n        Some(\"IV\") => ANNEX_B_IV,\n        Some(\"IW\") => ANNEX_B_IW,\n        _ => return None,\n    };\n    category.iter().find(|record| record.id.as_str() == id)\n}\n",
    );
    output
}

fn render_category(profile: &ValidatedProfile, category: &str) -> String {
    let mut output = header(profile);
    output.push_str(
        "use crate::runtime::{\n    AnnexBDecision, AnnexBId, AnnexBRecord, AnnexBValue, ApplicabilityStatus, DecisionStability,\n    DecisionVisibility,\n};\n\n",
    );
    writeln!(
        output,
        "/// Exact {category} Annex B records in runtime order.\n#[rustfmt::skip]\npub const ANNEX_B_{category}: &[AnnexBRecord] = &["
    )
    .expect("String writes cannot fail");
    for record in records(profile, category) {
        render_record(&mut output, profile, record);
    }
    output.push_str("];\n");
    output
}

fn records<'a>(
    profile: &'a ValidatedProfile,
    category: &str,
) -> Vec<&'a ImplementationDefinedChoiceRecord> {
    let mut records = profile
        .profile()
        .implementation_defined_choices
        .iter()
        .filter(|record| record.id.as_str().starts_with(category))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.runtime_order);
    records
}

fn render_record(
    output: &mut String,
    profile: &ValidatedProfile,
    record: &ImplementationDefinedChoiceRecord,
) {
    writeln!(output, "    AnnexBRecord {{").expect("String writes cannot fail");
    writeln!(
        output,
        "        id: AnnexBId::new({:?}),",
        record.id.as_str()
    )
    .expect("String writes cannot fail");
    writeln!(output, "        topic: {:?},", record.topic).expect("String writes cannot fail");
    writeln!(
        output,
        "        applicability: {:?},",
        record.applicability.as_str()
    )
    .expect("String writes cannot fail");
    writeln!(
        output,
        "        applicability_status: ApplicabilityStatus::{},",
        if profile
            .applicability(record.applicability.as_str())
            .expect("validated applicability")
        {
            "Applicable"
        } else {
            "NotApplicable"
        }
    )
    .expect("String writes cannot fail");
    output.push_str("        decision: ");
    render_decision(output, &record.decision);
    output.push_str(",\n        clause_anchors: &[");
    render_strings(output, record.clause_anchors.iter().map(|id| id.as_str()));
    output.push_str("],\n        evidence: &[");
    render_strings(output, record.evidence.iter().map(|id| id.as_str()));
    output.push_str("],\n    },\n");
}

fn render_decision(output: &mut String, decision: &ImplementationDefinedDecision) {
    match decision {
        ImplementationDefinedDecision::Selected {
            value,
            rationale,
            stability,
            visibility,
        } => {
            output.push_str("AnnexBDecision::Selected { value: ");
            render_value(output, value);
            write!(
                output,
                ", rationale: {rationale:?}, stability: DecisionStability::{}, visibility: DecisionVisibility::{} }}",
                stability_name(*stability),
                visibility_name(*visibility)
            )
            .expect("String writes cannot fail");
        }
        ImplementationDefinedDecision::Pending { owner, reason } => {
            write!(
                output,
                "AnnexBDecision::Pending {{ owner: {owner:?}, reason: {reason:?} }}"
            )
            .expect("String writes cannot fail");
        }
        ImplementationDefinedDecision::NotApplicable { reason } => {
            write!(
                output,
                "AnnexBDecision::NotApplicable {{ reason: {reason:?} }}"
            )
            .expect("String writes cannot fail");
        }
    }
}

fn render_value(output: &mut String, value: &ImplementationDefinedValue) {
    match value {
        ImplementationDefinedValue::Boolean { value } => {
            write!(output, "AnnexBValue::Boolean({value})")
        }
        ImplementationDefinedValue::UnsignedInteger { value } => {
            write!(output, "AnnexBValue::UnsignedInteger({value})")
        }
        ImplementationDefinedValue::Identifier { value } => {
            write!(output, "AnnexBValue::Identifier({value:?})")
        }
        ImplementationDefinedValue::String { value } => {
            write!(output, "AnnexBValue::String({value:?})")
        }
        ImplementationDefinedValue::OrderedIdentifierList { value } => {
            output.push_str("AnnexBValue::OrderedIdentifierList(&[");
            render_strings(output, value.iter().map(String::as_str));
            output.push_str("])");
            Ok(())
        }
        ImplementationDefinedValue::OrderedStringList { value } => {
            output.push_str("AnnexBValue::OrderedStringList(&[");
            render_strings(output, value.iter().map(String::as_str));
            output.push_str("])");
            Ok(())
        }
    }
    .expect("String writes cannot fail");
}

fn render_strings<'a>(output: &mut String, values: impl Iterator<Item = &'a str>) {
    for value in values {
        write!(output, "{value:?}, ").expect("String writes cannot fail");
    }
}

fn stability_name(value: DecisionStability) -> &'static str {
    match value {
        DecisionStability::Stable => "Stable",
        DecisionStability::Provisional => "Provisional",
    }
}

fn visibility_name(value: DecisionVisibility) -> &'static str {
    match value {
        DecisionVisibility::Public => "Public",
        DecisionVisibility::Embedder => "Embedder",
        DecisionVisibility::Internal => "Internal",
    }
}
