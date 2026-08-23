//! Command-line parsing and external output handling.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::run::{ClaimRequest, Decision, Manifest, Request, Selection, Shard, TRACE_PATH};
use super::{ConformanceError, Harness, invalid};

pub(super) fn run() -> Result<(), ConformanceError> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or_else(|| invalid("expected run or docs"))?;
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    if mode == "docs" {
        let operation = args
            .next()
            .ok_or_else(|| invalid("expected --check or --write"))?;
        parse_root_only(&mut args, &mut root)?;
        return match operation.as_str() {
            "--check" => write_or_check_trace(&root, false),
            "--write" => write_or_check_trace(&root, true),
            _ => Err(invalid("expected --check or --write")),
        };
    }
    if mode != "run" {
        return Err(invalid("expected run or docs"));
    }
    let (request, revision, output) = parse_run_args(args, &mut root)?;
    let harness = Harness::load(&root)?;
    let manifest = harness.run_claim(request, &revision, None)?;
    write_manifest(&root, &output, &manifest)?;
    if manifest.decision == Decision::Denied {
        return Err(invalid("claim denied; see external result manifest"));
    }
    Ok(())
}

pub(super) fn write_manifest(
    root: &Path,
    path: &Path,
    manifest: &Manifest,
) -> Result<(), ConformanceError> {
    if !path.is_absolute() {
        return Err(invalid("manifest output path must be absolute"));
    }
    let repository = root
        .canonicalize()
        .map_err(|source| io_error(root, source))?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("manifest output has no parent"))?;
    let parent = parent
        .canonicalize()
        .map_err(|source| io_error(parent, source))?;
    if parent.starts_with(repository) {
        return Err(invalid("manifest output must be outside the repository"));
    }
    let mut bytes = serde_json::to_vec_pretty(manifest)?;
    bytes.push(b'\n');
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    output
        .write_all(&bytes)
        .map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: std::io::Error) -> ConformanceError {
    ConformanceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn write_or_check_trace(root: &Path, write: bool) -> Result<(), ConformanceError> {
    let harness = Harness::load(root)?;
    let path = root.join(TRACE_PATH);
    let expected = harness.render_traceability();
    if write {
        return fs::write(&path, expected).map_err(|source| io_error(&path, source));
    }
    let actual = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
    if actual != expected {
        return Err(invalid(format!(
            "generated traceability is stale: {}",
            path.display()
        )));
    }
    Ok(())
}

fn parse_root_only(
    args: &mut impl Iterator<Item = String>,
    root: &mut PathBuf,
) -> Result<(), ConformanceError> {
    while let Some(argument) = args.next() {
        if argument != "--root" {
            return Err(invalid(format!("unknown argument {argument}")));
        }
        *root = PathBuf::from(
            args.next()
                .ok_or_else(|| invalid("--root requires a path"))?,
        );
    }
    Ok(())
}

fn parse_run_args(
    mut args: impl Iterator<Item = String>,
    root: &mut PathBuf,
) -> Result<(Request, String, PathBuf), ConformanceError> {
    let mut claim = None;
    let mut revision = None;
    let mut output = None;
    let mut selection = Selection::default();
    let mut shard = Shard::default();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| invalid(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--root" => *root = PathBuf::from(value),
            "--claim" => claim = Some(ClaimRequest::parse(&value)?),
            "--revision" => revision = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--rule" => selection.rule = Some(value),
            "--feature" => selection.feature = Some(value),
            "--clause" => selection.clause = Some(value),
            "--owner" => selection.owner_pr = Some(value),
            "--shard-index" => {
                shard.index = value.parse().map_err(|_| invalid("invalid shard index"))?;
            }
            "--shard-count" => {
                shard.count = value.parse().map_err(|_| invalid("invalid shard count"))?;
            }
            _ => return Err(invalid(format!("unknown argument {flag}"))),
        }
    }
    Ok((
        Request {
            claim: claim.ok_or_else(|| invalid("--claim is required"))?,
            selection,
            shard,
        },
        revision.ok_or_else(|| invalid("--revision is required"))?,
        output.ok_or_else(|| invalid("--output is required"))?,
    ))
}
