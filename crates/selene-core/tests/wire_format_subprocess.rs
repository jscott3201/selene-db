//! Cross-process wire-format checks for IStr-keyed containers.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{PropertyMap, Value, intern};

#[test]
fn property_map_wire_bytes_are_independent_of_admission_order() {
    let unique = unique_prefix();
    let apple = format!("{unique}.apple");
    let banana = format!("{unique}.banana");
    let zebra = format!("{unique}.zebra");
    let first_order = [zebra.as_str(), apple.as_str(), banana.as_str()].join(",");
    let second_order = [banana.as_str(), zebra.as_str(), apple.as_str()].join(",");
    let dir = env::temp_dir();
    let first_path = dir.join(format!("{unique}.first.postcard"));
    let second_path = dir.join(format!("{unique}.second.postcard"));

    run_child(&first_order, &first_path);
    run_child(&second_order, &second_path);

    let first = fs::read(&first_path).expect("read first child output");
    let second = fs::read(&second_path).expect("read second child output");
    assert_eq!(
        first, second,
        "canonical wire order must not depend on IStr admission order"
    );
    let _ = fs::remove_file(first_path);
    let _ = fs::remove_file(second_path);
}

#[test]
fn wire_format_child_entry() {
    let Some(output) = env::var_os("SELENE_WIRE_CHILD_OUTPUT") else {
        return;
    };
    let order = env::var("SELENE_WIRE_CHILD_ORDER").expect("child order env set");
    let names: Vec<_> = order.split(',').collect();
    assert_eq!(names.len(), 3, "test child expects three keys");
    for name in &names {
        intern(name).expect("child admits key in requested order");
    }

    let mut logical_keys = names.clone();
    logical_keys.sort_unstable();
    let map = PropertyMap::from_pairs([
        (intern(logical_keys[0]).unwrap(), Value::Int(1)),
        (intern(logical_keys[1]).unwrap(), Value::Int(2)),
        (intern(logical_keys[2]).unwrap(), Value::Int(3)),
    ])
    .expect("map builds");
    let bytes = postcard::to_allocvec(&map).expect("map serializes");
    fs::write(output, bytes).expect("child writes output");
}

fn run_child(order: &str, output: &Path) {
    let status = Command::new(env::current_exe().expect("current test exe"))
        .arg("--exact")
        .arg("wire_format_child_entry")
        .arg("--nocapture")
        .env("SELENE_WIRE_CHILD_ORDER", order)
        .env("SELENE_WIRE_CHILD_OUTPUT", output)
        .status()
        .expect("child test process starts");
    assert!(status.success(), "child test process failed: {status}");
}

fn unique_prefix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!("brief92-wire-{}-{nanos}", std::process::id())
}
