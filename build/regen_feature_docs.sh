#!/usr/bin/env bash
set -euo pipefail

# Placeholder generator for BRIEF-02.
#
# Intended source of truth:
#   crates/selene-core/src/feature_register.rs
#
# Intended generated/check targets:
#   _spec/01-mission-and-conformance-target.md section 5.1 / section 5.3 / section 7
#   _spec/07-iso-gql-parser-and-flagger.md section 7.3
#   _spec/09-engineering-discipline.md section 5
#
# Manual checklist until selene-core exists as a real crate:
#   1. SUPPORTED_FEATURES matches ISO Annex A names and Annex D rows.
#   2. Spec 01 supported/not-supported prose names the same feature IDs.
#   3. Spec 07 references selene_core::feature_register::SUPPORTED_FEATURES.
#   4. Spec 09 summarizes ANNEX_B_REGISTER rather than defining a second list.
#   5. No prose-only feature ID claim exists without a FeatureId constant.

echo "TODO: generate and diff conformance docs from feature_register.rs" >&2
exit 0
