# GQL conformance and claim policy

The implementation target is ISO/IEC 39075:2024, first edition. The licensed
publication remains authoritative. Repository artifacts may carry clause,
feature, rule, and implementation-defined identifiers with paraphrased choices
and evidence; they must not reproduce the publication.

## Current posture

The c5 feature register reports implemented and parser-visible surface. It is
not sufficient evidence for a formal 2.0 conformance claim. M01 establishes the
generated, implication-closed profile authority. Until that gate exists and is
green, public wording is limited to “GQL-oriented” or “implements selected GQL
syntax and semantics,” with known gaps.

## Claim states

| Evidence state | Maximum public wording |
|---|---|
| Registry/profile incomplete | GQL-oriented or selected GQL syntax/semantics, with gaps. |
| Profile/evidence present, blockers remain | Aligned with ISO/IEC 39075:2024, accompanied by the generated profile, unsupported features, blockers, and extensions. |
| Selected profile implication-closed and complete; all evidence green | Generated wording that names the exact implementation, profile, features, and property types. |

Manual release prose must not exceed the generated claim.

## M01 authority

The future `selene-profile` source and generated artifacts will own feature
taxonomy and implications, implementation-defined choices and applicability,
implementation-dependent disclosures, Unicode/collation/source repertoire,
extension inventory, flagger state, runtime status, rule/evidence traceability,
and the release declaration. That crate and authority do not exist at the c5
baseline.

## Evidence bar

A feature is not complete because syntax parses or one successful case passes.
Applicable evidence covers positive results, negative syntax/access/type
behavior, primary and additional GQLSTATUS data, declared type and nullability,
duplicates and order, catalog/data/session/transaction effects, identity and
invalidation, durability and recovery, differential or model tests, and
mutation/fuzz evidence for high-risk parsers, decoders, or state machines.

Every applicable implementation-defined item needs a typed choice, rationale,
clause occurrence map, user disclosure, and evidence. Non-applicability is
explicit. Namespaced vectors, indexes, constraints, text, JSON, algorithms,
and administrative procedures remain Selene extensions where they are not ISO
features.
