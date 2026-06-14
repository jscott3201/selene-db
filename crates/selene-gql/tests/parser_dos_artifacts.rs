//! PARSER-DOS regression: the parser complexity guard must reject the
//! super-linear pest-backtracking artifacts in well-bounded time.
//!
//! # Background
//!
//! pest is not packrat-memoized, so a run of *unclosed* `[` openers can drive
//! expensive nested list-literal expression attempts — seconds-to-minutes for
//! sub-kilobyte inputs before the dedicated guard existed. Parsing precedes
//! execution, so an execution deadline cannot interrupt the blow-up; the
//! pre-pest byte-scan guard bounds the *depth of simultaneously-open `[`*
//! instead (these artifacts are pure unclosed `[`, so their `[`-depth equals
//! their opener count).
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

use std::time::Duration;

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

/// `fuzz/artifacts/parse_gql/timeout-0566cc3ad711f03d74b6f489a35db4cf806e1363` (795 bytes).
const CASE_PREFIX_TIMEOUT_0566CC3AD: &[u8] = b"// ck\nRETURN CQSeiv/re-ok\nRETURN CASeaturETURN *har_leGV56s56siCASe--------CASeaturETURN *har-----CASeaturETURN *har_leGV57siCASea--r_leleGV56siCASea------------2----------------------------CASeaturETURN *har-----CASeiCASea--------------2------------2----------------------CASeaturETURN *har_leGV57siCASea--r_leleGV56siCASea------------2----------------------------CASeaturETURN *har-----CASeaturETURN *har_leGViCASea--r_leleGV56siCASea------------2-------------------------------CASeaturETURN *har-[----CASeat---------CASeaturETURN *har-----CASeaturETURN *har_leGViCASea--r_leleGV56siCASea------------2-------------------------------CASeaturETURN *har-[----CASeaturETURN *har_leGV56siCASea--r_le------UR R";

/// `fuzz/artifacts/parse_gql/timeout-88d2b5bc2a7081df2b2c077b20f70540b3f8d489` (1486 bytes).
const TRIM_TIMEOUT_88D2B5BC_HEX: &str = concat!(
    "2f2f20636f727075733a20706f7369746976650a2f2f6374733a207061284c454144494e47202778272046524f4d2027787868656c6c6f782729207273652d6f6b0a52455455524e2031204153206e20494e544552534563542052455455524e",
    "20322d696d6d65642c0a202020202020205452494d282f2f70615b4431207372652c4052534563542052455455524e20322d3c3d4563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c",
    "4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20270100000000000029787868656c6c6f78272920412032",
    "2c20782729204120322c205455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d52534563542052455455524e20322d696d6d65642c0a202020202020205452",
    "494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20277878685320764c",
    "454144494e47202778272046616c524f4d2027787868656c6c6f782729204153206c656164696e675f22222222227472696d6d65642c52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f705320764c27",
    "787868656c6c6f782729204153206c6561696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d2027787868656c6c66782729204120322c205455524e20",
    "322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d5253456354205245542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f70617273654420",
    "5b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20270100000000000029787868656c6c6f78272920",
    "4120322c20782729204120322c205455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d52534563542052455455524e20322d696d6d65642c0a202020202020",
    "205452494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f7061727355524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c",
    "4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20277878685320764c454144494e47202778272046616c52",
    "4f4d2027787868656c6c6f782729204153206c656164696e675f22222222227472696d6d65642c52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f70615b4431207372652c4052534563542052455455",
    "524e20322d3c3d4563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b",
    "312c20322c205320764c454144494e47202778272046616c524f4d2027787868656c6c6f782729204120322c205320764c27787868656c6c6f782729204153206c6561696d6d65642c0a202020202020205452494d282f2f706172736544200a",
    "202020202020205452494628545241494c27784e491c472720465275650a52455455524e2073744f4d2027204153",
);

/// `fuzz/artifacts/parse_gql/slow-unit-b80e9edac0b6963af1eee48576b733839ccdcd8f` (1265 bytes).
const TRIM_SLOW_B80E9EDA_HEX: &str = concat!(
    "2f2f20636f727075733a20706f7369746976650a2f2f6374733a207061284c454144494e47202778272046524f4d2027787868656c6c6f782729207273652d702f0a52455455524e2031204153206e20494e544552534563542052455455524e",
    "20322d696d6d65642c0a202020202020205452494d282f2f70615b4431207372652c4052534563542052455455524e20322d3c3d4563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c",
    "4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d2027787868656c6c6f7827524e20322d696d6d65642c0a20",
    "2020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f70617229204120322c20782729204120322c205455524e20322d696d6d65642c0a202020",
    "202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e",
    "20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20277878685320764c454144494e47202778272046616c524f4d2027787868656c6c6f7827",
    "29204153206c656164696e675f22222222227472696d6d65642c52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f705320764c27787868656c6c6f782729204153206c6561696d6d65642c0a20202020",
    "2020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d2027787868656c6c66782729204120322c205455524e20322d696d6d65642c0a202020202020205452494d282f2f7061727365",
    "44205b312c4052534563542052455455524e20322d52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020",
    "205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d20277878685320764c454144494e47202778273f46616c524f4d2027787868656c6c6f782729204153206c656164696e675f222222",
    "22227472696d6d65642c52534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f70615b4431207372652c4052534563542052455455524e20322d3c3d4563542052455455524e20322d696d6d65642c0a2020",
    "20202020205452494d282f2f706172736544205b312c4052534563542052455455524e20322d696d6d65642c0a202020202020205452494d282f2f706172736544205b312c20322c205320764c454144494e47202778272046616c524f4d2027",
    "787868656c6c6f782729204120322c205320764c27787868656c6c6f782729204153206c6561696d6d65642c0a202020202020205452494d282f2f706172736544200a202020202020205452494d28545241494c27784e491c47272046527565",
    "0a52455455524e2073744f4d2027204153",
);

const TRIM_FALSE_START_ARTIFACTS: &[(&str, &str)] = &[
    ("timeout-88d2b5bc", TRIM_TIMEOUT_88D2B5BC_HEX),
    ("slow-unit-b80e9eda", TRIM_SLOW_B80E9EDA_HEX),
];

/// Per-artifact wall-clock ceiling — a coarse, deliberately loose tripwire.
///
/// The *primary*, deterministic guarantee is structural and lives in
/// `artifacts_reject_with_program_limit_exceeded_status`: each artifact is
/// rejected by the pre-pest byte-scan guard (`ComplexityLimitExceeded`), so
/// pest is never reached and the real cost is sub-microsecond. That assertion
/// does not depend on wall-clock and cannot flake.
///
/// This timing check is only a secondary backstop against a hypothetical future
/// change that reintroduces the catastrophic super-linear blow-up (the original
/// was seconds-to-minutes). Because `Instant::elapsed` includes scheduler
/// pauses — and these tests run alongside dozens of other gql binaries under
/// nextest, where a transient deschedule on a throttled CI runner can stall a
/// thread for tens of milliseconds — the budget is set generously at 250ms.
/// That is ~25,000x today's sub-microsecond cost (immune to realistic CI
/// jitter) while still tripping by 10–100x+ on a multi-second regression. A
/// broken guard that lets an artifact reach pest is caught deterministically by
/// the structural test above (the error type changes), not by this timing.
const PARSE_BUDGET: Duration = Duration::from_millis(250);

/// The PARSER-DOS complexity-guard regression module: artifact replay plus the
/// boundary unit cases relocated out of `parser/mod.rs` to keep that near-cap
/// module file under the 700-LOC gate while consolidating every complexity
/// regression in one place.
mod dos {
    use super::{ARTIFACTS, PARSE_BUDGET};
    use std::time::Instant;

    use selene_gql::{GqlStatus, ParserError, parse};

    /// Net-nesting cap (`guard::MAX_NESTING_DEPTH`); the public crate does not
    /// re-export the parser constant, so it is mirrored here. A `[` run one
    /// longer than this is well above the dedicated `[`-depth complexity cap of
    /// 32 and must be rejected by the complexity guard before pest runs.
    const NESTING_CAP: usize = 64;

    #[test]
    fn bracket_complexity_limit_enforced_on_artifacts() {
        for (label, bytes) in ARTIFACTS {
            // The fuzz target feeds raw bytes through `str::from_utf8` first;
            // every artifact is valid UTF-8, so this mirrors the fuzzer's path.
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

            // Contract: "parse or reject". Every artifact nests 56-159 unclosed
            // `[` (far above the `[`-depth cap of 32), so today each is
            // rejected; a future change that lets one parse is acceptable only
            // if it stays within PARSE_BUDGET above. The one thing that is never
            // acceptable is a rejection reached *after* expensive recursive
            // descent, so any error MUST be the pre-pest complexity guard.
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
        // Belt-and-suspenders on the GQLSTATUS mapping, consistent with the
        // "parse or reject" contract: if an artifact is rejected, the rejection
        // must be the implementation-defined 5GQL1 PROGRAM_LIMIT_EXCEEDED class
        // (ISO/IEC 39075:2024 section 23.1), never 42001 SYNTAX_ERROR. A
        // hypothetical future fast-parse path that accepts an artifact is not a
        // failure of this assertion (it is covered by the timing test above).
        for (label, bytes) in ARTIFACTS {
            let source = std::str::from_utf8(bytes).expect("artifact is valid UTF-8");
            if let Err(error) = parse(source) {
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
    }

    #[test]
    fn bracket_complexity_limit_maps_to_program_limit_exceeded() {
        // A run of `[` nested past the dedicated `[`-depth complexity cap (and
        // here also past the general nesting cap) is reported as the more
        // precise complexity violation, mapping to GQLSTATUS 5GQL1
        // PROGRAM_LIMIT_EXCEEDED.
        let over_cap = "[".repeat(NESTING_CAP + 1);
        let source = format!("RETURN {over_cap}0");
        let error = parse(&source).expect_err("a deep bracket run exceeds the complexity cap");
        assert!(
            matches!(error, ParserError::ComplexityLimitExceeded { .. }),
            "expected ComplexityLimitExceeded, got {error:?}"
        );
        assert_eq!(error.gqlstatus(), GqlStatus::PROGRAM_LIMIT_EXCEEDED);
    }

    #[test]
    fn bracket_complexity_allows_legitimate_boundary_queries() {
        // The complexity cap bounds `[` *nesting depth* (32), not a total opener
        // count, so deeply nested `[` is the only thing it rejects. The deepest
        // legitimate `[` nesting in the workspace is 3 (`[[[1]]]`).
        let nine_call_return = "RETURN sin(0), cos(0), tan(0), cot(1), asin(0), \
             acos(1), atan(0), degrees(3.0), radians(180)";
        parse(nine_call_return).expect("a 9-call RETURN nests no list brackets");

        // A nested list literal three levels deep and a record with two lists
        // both sit far below the `[`-depth cap.
        parse("RETURN [[[1]]]").expect("a depth-3 list literal parses");
        parse("RETURN {a: [1, 2], b: [3, 4]}").expect("a record of two lists parses");

        // Regression guard for the false-positive a total-opener-count cap would
        // hit: a long fixed path has many balanced `(`/`[` openers (here 31 `(`
        // plus 30 `[` = 61 openers, more than the smallest hostile artifact's
        // 56) but `[`-depth 1 — each edge bracket closes before the next opens —
        // so it must parse no matter how many hops.
        let mut long_path = String::from("MATCH (n0)");
        for hop in 1..=30 {
            long_path.push_str(&format!("-[:E]->(n{hop})"));
        }
        long_path.push_str(" RETURN n0");
        parse(&long_path).expect("a 30-hop fixed path has list-bracket depth 1 and must parse");
    }
}

/// BRIEF 01b: zero-delimiter deep-recursion crash regression.
///
/// Two `grammar.pest` rules recurse toward `expr` with no guarded delimiter
/// (`(`/`[`/`{`) AND a token the byte-scan can soundly recognize: `unary` (a run
/// of unary `+`/`-` signs) and `not_expr` (a run of reserved-keyword `NOT`). A
/// long enough run drives pest's recursive descent off the native stack — a
/// non-unwindable
/// SIGABRT that hard-kills the host process embedding selene-db, because
/// `catch_unwind` cannot trap a stack overflow. The pre-pest byte-scan guard
/// (`guard::validate`, `MAX_RECURSION_DEPTH = 256`) rejects each run
/// deterministically with `ComplexityLimitExceeded` (GQLSTATUS 5GQL1) **before**
/// recursive descent begins.
///
/// # These tests MUST NOT crash the runner
///
/// nextest runs each test on the default ~2 MB thread stack. On that stack the
/// sign chain overflows pest at ~1000-2000 signs, so an over-floor input run
/// *through pest* would SIGABRT the entire test binary (taking every other test
/// with it). The cap of 256 is a deliberate safety floor ~4-8x below that
/// overflow point: an input at `cap + 1` (257) is rejected by the byte-scan
/// guard before pest sees it, so it is safe to replay here. Every assertion
/// below checks the *structural pre-pest rejection by error variant* and never
/// relies on pest surviving an over-floor descent.
mod recursion_crash {
    use selene_gql::{GqlStatus, ParserError, parse};

    /// `guard::MAX_RECURSION_DEPTH` (the crate does not re-export the parser
    /// constant). An input one deeper than this must be rejected pre-pest.
    const RECURSION_CAP: usize = 256;

    /// A count safely above the cap but far below the ~2 MB-thread pest overflow
    /// floor (~1000-2000), so replaying it can never crash the runner: the guard
    /// rejects it structurally before pest is reached.
    const OVER_CAP: usize = RECURSION_CAP + 64;

    /// Assert `source` is rejected by the pre-pest guard with the deterministic
    /// complexity error (never a post-descent syntax error, never a crash).
    fn assert_rejected_pre_pest(label: &str, source: &str) {
        let error = parse(source).expect_err(label);
        assert!(
            matches!(error, ParserError::ComplexityLimitExceeded { .. }),
            "{label}: expected ComplexityLimitExceeded (pre-pest), got {error:?}"
        );
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::PROGRAM_LIMIT_EXCEEDED,
            "{label}: rejection must carry 5GQL1 PROGRAM_LIMIT_EXCEEDED"
        );
    }

    #[test]
    fn unary_sign_chain_rejected_pre_pest() {
        // Vector 1: `RETURN ----...1` — `unary = { sign_op ~ unary | postfix }`.
        let chain = "-".repeat(OVER_CAP);
        let source = format!("RETURN {chain}1");
        assert_rejected_pre_pest("unary sign chain", &source);
    }

    #[test]
    fn mixed_sign_chain_rejected_pre_pest() {
        // A `+`/`-` mix is still one maximal `sign_run`.
        let chain = "+-".repeat(OVER_CAP); // 2*OVER_CAP signs, all consecutive
        let source = format!("RETURN {chain}1");
        assert_rejected_pre_pest("mixed sign chain", &source);
    }

    #[test]
    fn sign_chain_with_interleaved_comments_rejected_pre_pest() {
        // Comments are whitespace to pest, so `- /* c */ -` is still a 2-deep
        // unary chain: the guard must NOT reset the run on a comment. A chain of
        // signs separated only by block comments must still trip the cap.
        let mut chain = String::new();
        for _ in 0..OVER_CAP {
            chain.push_str("- /* c */ ");
        }
        let source = format!("RETURN {chain}1");
        assert_rejected_pre_pest("comment-separated sign chain", &source);
    }

    #[test]
    fn not_keyword_chain_rejected_pre_pest() {
        // Vector 2: `RETURN NOT NOT ... true` — `not_expr = { not_kw ~ not_expr
        // | is_expr }`. Whole-word, case-insensitive recognition.
        let chain = "NOT ".repeat(OVER_CAP);
        let source = format!("RETURN {chain}true");
        assert_rejected_pre_pest("NOT keyword chain", &source);
    }

    #[test]
    fn lowercase_not_chain_rejected_pre_pest() {
        // The keyword recognition is case-insensitive (`^"NOT"` in the grammar).
        let chain = "not ".repeat(OVER_CAP);
        let source = format!("RETURN {chain}true");
        assert_rejected_pre_pest("lowercase not chain", &source);
    }

    #[test]
    fn note_and_notnull_do_not_match_not() {
        // `NOTE`/`NOTNULL` are distinct whole words and must NOT increment the
        // `NOT` counter. A long run of `NOTE` identifiers (here used as RETURN
        // aliases) must not be rejected as a `NOT` chain. They reset the run, so
        // even far past the cap the guard admits the structure to pest, which
        // then rejects it as a plain syntax error (NOT a complexity error).
        let words = (0..OVER_CAP)
            .map(|index| format!("NOTE{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!("RETURN {words}");
        // A run of bare identifiers is a syntax error at pest; that is fine — the
        // point is it must NOT be the complexity guard (which would mean `NOTE`
        // was misread as `NOT`).
        if let Err(ParserError::ComplexityLimitExceeded { .. }) = parse(&source) {
            panic!("a run of NOTE identifiers must not be misread as a NOT chain");
        }
    }

    #[test]
    fn case_keyword_prefix_false_starts_reject_quickly() {
        // v1.2.0 release fuzz found this malformed input: many identifier words
        // begin with `CASE` (`CASeaturETURN`) and used to send pest into a
        // near-timeout false-start path. The parser must reject it before that
        // prefix backtracking becomes observable.
        use std::time::Instant;

        let source = std::str::from_utf8(super::CASE_PREFIX_TIMEOUT_0566CC3AD)
            .expect("fuzz artifact must be valid UTF-8");
        let start = Instant::now();
        let result = parse(source);
        let elapsed = start.elapsed();

        assert!(
            elapsed < super::PARSE_BUDGET,
            "CASE-prefix fuzz artifact took {elapsed:?} to parse (budget {:?})",
            super::PARSE_BUDGET
        );
        assert!(
            result.is_err(),
            "CASE-prefix fuzz artifact is malformed and must not parse"
        );
    }

    #[test]
    fn trim_keyword_false_start_artifacts_reject_quickly() {
        // The June 13, 2026 nightly fuzz soak found malformed inputs with many
        // nested `TRIM(` false starts after line comments. These used to drive
        // pest into multi-second backtracking even though they are not deep
        // enough to trip the structural complexity guard.
        use std::time::Instant;

        for (label, hex) in super::TRIM_FALSE_START_ARTIFACTS {
            let bytes = decode_hex(label, hex);
            let source = std::str::from_utf8(&bytes)
                .unwrap_or_else(|error| panic!("{label} must be valid UTF-8: {error}"));
            let start = Instant::now();
            let result = parse(source);
            let elapsed = start.elapsed();

            assert!(
                elapsed < super::PARSE_BUDGET,
                "TRIM false-start artifact {label} took {elapsed:?} to parse (budget {:?})",
                super::PARSE_BUDGET
            );
            assert!(
                result.is_err(),
                "TRIM false-start artifact {label} is malformed and must not parse"
            );
        }
    }

    fn decode_hex(label: &str, hex: &str) -> Vec<u8> {
        assert_eq!(
            hex.len() % 2,
            0,
            "{label}: embedded artifact hex must have even length"
        );
        hex.as_bytes()
            .chunks_exact(2)
            .enumerate()
            .map(|(index, pair)| {
                let high = hex_nibble(pair[0])
                    .unwrap_or_else(|| panic!("{label}: invalid high hex digit at {index}"));
                let low = hex_nibble(pair[1])
                    .unwrap_or_else(|| panic!("{label}: invalid low hex digit at {index}"));
                (high << 4) | low
            })
            .collect()
    }

    fn hex_nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    #[test]
    fn flat_wide_return_is_linear_and_admitted() {
        // AC#9 (parity): flat, wide RETURN lists pin Lens-C linearity. The
        // parser must admit N distinct identifiers cleanly and stay linear in
        // N (no super-linear blow-up, no false complexity rejection).
        // This pins both the cardinality (every item makes it into the AST) and
        // a loose timing tripwire.
        use std::time::{Duration, Instant};

        // Wide but well within all caps (flat, zero-delimiter, zero recursion).
        let n = 2_000;
        let items = (0..n)
            .map(|index| format!("item{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("RETURN {items}");

        let start = Instant::now();
        let statement = parse(&source).expect("a wide flat RETURN parses without a budget cap");
        let elapsed = start.elapsed();

        // Loose tripwire: generous against CI jitter, but a super-linear
        // regression on a 2000-item flat list would blow well past it. Ubuntu
        // release runners have measured above 1s for this unoptimized test.
        let budget = Duration::from_secs(2);
        assert!(
            elapsed < budget,
            "flat RETURN of {n} items took {elapsed:?} (budget {budget:?}); parse cost should be ~linear in N"
        );

        // Cardinality: all N projection items survive into the AST (linearity is
        // meaningless if items are silently dropped).
        let selene_gql::Statement::Query(query) = statement else {
            panic!("expected a query statement");
        };
        let selene_gql::PipelineStatement::Return(clause) = &query.statements[0] else {
            panic!("expected a RETURN clause");
        };
        assert_eq!(
            clause.items.len(),
            n,
            "every projection item must be parsed"
        );
    }
}
