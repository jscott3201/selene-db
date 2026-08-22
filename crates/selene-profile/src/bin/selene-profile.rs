//! Explicit checked-in profile generator and freshness checker.

use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or("expected --check or --write")?;
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();

    while let Some(argument) = args.next() {
        if argument == "--root" {
            root = PathBuf::from(args.next().ok_or("--root requires a path")?);
        } else {
            return Err(format!("unknown argument: {}", argument.to_string_lossy()).into());
        }
    }

    if mode == "--check" {
        selene_profile::check_repository(&root)?;
    } else if mode == "--write" {
        selene_profile::write_repository(&root)?;
    } else {
        return Err("expected --check or --write".into());
    }
    Ok(())
}
