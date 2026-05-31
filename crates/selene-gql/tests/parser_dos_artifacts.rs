//! PARSER-DOS regression: the parser complexity guard must reject the
//! super-linear pest-backtracking artifacts in well-bounded time.
//!
//! # Background
//!
//! pest is not packrat-memoized, so a wide, shallow fan-out of `[` openers
//! re-explored under the three `[`-prefixed expression rules
//! (`list_access_op`, `list_comprehension`, `list_lit`) drives super-linear
//! parse time — seconds-to-minutes for sub-kilobyte inputs — even though net
//! nesting depth stays under the 64-delimiter nesting cap. Parsing precedes
//! execution, so an execution deadline cannot interrupt the blow-up; the
//! pre-pest byte-scan guard bounds the *total* opener count instead.
//!
//! These six artifacts were surfaced by the v1.1.0 release fuzz soak. They are
//! gitignored under `fuzz/artifacts/parse_gql/` (the whole artifacts tree is
//! ignored), so a fresh clone / CI checkout does not have the files. Their
//! exact bytes are embedded here so the regression is hermetic and runs in the
//! standard `nextest` gate without the fuzz corpus on disk.
//!
//! The bar is "would this catch a re-introduction of the backtracking
//! blow-up": each artifact must parse-or-reject in well under a fixed budget,
//! and any rejection must be the complexity guard (GQLSTATUS 5GQL1
//! PROGRAM_LIMIT_EXCEEDED per ISO/IEC 39075:2024 §23.1), never a syntax error
//! reached after recursive descent.

use std::time::{Duration, Instant};

use selene_gql::{ParserError, parse};

// Each artifact is valid UTF-8 and uses only `[` openers (no `{`/`(`). The
// trailing `\xd8\xb1` (a 2-byte UTF-8 ARABIC LETTER REH) and embedded control
// bytes are reproduced exactly from the on-disk corpus so the regression
// matches the fuzzer's literal input.

/// `fuzz/artifacts/parse_gql/slow-unit-3080527b8db1b09935d4e16c640a036c2e687d93` (470 bytes, 124 openers).
const SLOW_3080527B: &[u8] = b"lETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[sETf[sETf[[[[CAsETQ[[esETf[[[[CAsETQf[sETf[[9[esETf[[[CAsETQf[sETf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETETQf[[esETf[[[[CAsETQf.Enn556esETf[[[CAs[ETQf.Enn55>[esETf[[[[CAsETQf[sETf[sETf[[[[CAsETQ[[esETf[[[[CAsETQf[sETf[[9[esETf[[[CAsETQf[sETf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[.<[005ETQ'\\LLL*-[5\xd8\xb10";

/// `fuzz/artifacts/parse_gql/slow-unit-9a3e047559e31119563304366654f08b657c1b0d` (404 bytes, 104 openers).
const SLOW_9A3E0475: &[u8] = b"lETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.ETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.Enn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETnn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETf[Qf[[ACsE[[TQ\x1cf[sETE.E[005ETQ'\\LLL*-[5\xd8\xb10";

/// `fuzz/artifacts/parse_gql/slow-unit-e14d200a024d6d5bffb71fde35e4ad1b2ed36c07` (262 bytes, 57 openers).
const SLOW_E14D200A: &[u8] = b"lETl00000001=E.EnnTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECAsETTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CAsETQQf[sETE.EQf[sETE.nnTf[[[[CAsECAsETQ[[[[[CA[[CAsECAsETQ[[CAsETQQf[sETE.EQf[sETE.ECETf[[[E000\x00\x00\x00\x00\x00\x00\x000.";

/// `fuzz/artifacts/parse_gql/slow-unit-764959657ebaeda78ec5014f00cfe47a1be01fbd` (254 bytes, 56 openers).
const SLOW_764959657: &[u8] = b"lETl00000001=E.EnnTf%[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECAsETTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[sETQ[[CA0001=E.EnnTf[[[[CAsECAsETTf[[[[CAsECAsETQ[[[[[CAsECAsETQ[[CA0001=E.EnnTf[[[[CAsECQ[[[[[CAsECAsETQ[[CAsETQQf[sETE.EQf[sETE.ECETETE =0is";

/// `fuzz/artifacts/parse_gql/slow-unit-f3a4d77659bd5327589e793c107f98bbddc12233` (579 bytes, 159 openers).
const SLOW_F3A4D77: &[u8] = b"lETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.ETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.Enn5esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETnn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[656[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETnn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETf[Qf[[ACsE[[TQ\x1cf[sETE.E[005ETQ'\\LLL*-[5\xd8\xb10";

/// `fuzz/artifacts/parse_gql/timeout-f3a4d77659bd5327589e793c107f98bbddc12233` (579 bytes, 159 openers).
const TIMEOUT_F3A4D77: &[u8] = b"lETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.ETl00001=E556[esETf[[[[CAsETQf.Enn556esETf[[[[CAsETQf.Enn5esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETnn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[656[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETnn556[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[s<Tf[[[[esETf[[[[CAsETQf[sETf[[6[esETf[[[[CAsETQf[sETf[[[[CAsETQf[sETf[[6[esETf[Qf[[ACsE[[TQ\x1cf[sETE.E[005ETQ'\\LLL*-[5\xd8\xb10";

/// All six fuzz artifacts, paired with a stable label for failure diagnostics.
const ARTIFACTS: &[(&str, &[u8])] = &[
    ("slow-unit-3080527b", SLOW_3080527B),
    ("slow-unit-9a3e0475", SLOW_9A3E0475),
    ("slow-unit-e14d200a", SLOW_E14D200A),
    ("slow-unit-764959657", SLOW_764959657),
    ("slow-unit-f3a4d77", SLOW_F3A4D77),
    ("timeout-f3a4d77", TIMEOUT_F3A4D77),
];

/// Per-artifact wall-clock ceiling. Today's worst case post-fix is sub-
/// microsecond (the guard rejects before pest runs); 50ms is robust headroom
/// under heavy build/test parallelism while still catching a regression that
/// reintroduces the seconds-to-minutes blow-up.
const PARSE_BUDGET: Duration = Duration::from_millis(50);

#[test]
fn bracket_complexity_limit_enforced_on_artifacts() {
    for (label, bytes) in ARTIFACTS {
        // The fuzz target feeds raw bytes through `str::from_utf8` first; every
        // artifact is valid UTF-8, so this mirrors the fuzzer's code path.
        let source = std::str::from_utf8(bytes)
            .unwrap_or_else(|error| panic!("artifact {label} must be valid UTF-8: {error}"));

        let start = Instant::now();
        let result = parse(source);
        let elapsed = start.elapsed();

        assert!(
            elapsed < PARSE_BUDGET,
            "artifact {label} took {elapsed:?} to parse (budget {PARSE_BUDGET:?}); the \
             super-linear bracket-backtracking blow-up has regressed"
        );

        // The guard must reject every artifact (each opens 56-159 brackets,
        // far above the cap of 20). A future change that lets one parse is
        // acceptable only if it is genuinely fast, so accept Ok as well — but
        // any rejection MUST be the complexity guard, never a syntax error
        // reached after expensive recursive descent.
        match result {
            Ok(_) => {}
            Err(ParserError::ComplexityLimitExceeded { .. }) => {}
            Err(other) => panic!(
                "artifact {label} rejected with {other:?} (status {}); expected \
                 ComplexityLimitExceeded (5GQL1)",
                other.gqlstatus()
            ),
        }
    }
}

#[test]
fn artifacts_reject_with_program_limit_exceeded_status() {
    // Belt-and-suspenders on the GQLSTATUS mapping: rejection is the
    // implementation-defined 5GQL1 PROGRAM_LIMIT_EXCEEDED class (ISO/IEC
    // 39075:2024 §23.1), not 42001 SYNTAX_ERROR.
    for (label, bytes) in ARTIFACTS {
        let source = std::str::from_utf8(bytes).expect("artifact is valid UTF-8");
        let error = parse(source).expect_err(&format!(
            "artifact {label} opens far more brackets than the cap"
        ));
        assert!(
            matches!(error, ParserError::ComplexityLimitExceeded { .. }),
            "artifact {label}: expected ComplexityLimitExceeded, got {error:?}"
        );
        assert_eq!(
            error.gqlstatus().as_str(),
            "5GQL1",
            "artifact {label}: rejection must carry PROGRAM_LIMIT_EXCEEDED",
        );
    }
}
