use super::*;

#[test]
fn apply_node_create_admits_string_into_string_index() {
    // `Value::String` reaches the property-index commit and produces a
    // findable index entry.
    let label = db_string("pi.string.create.label").unwrap();
    let name = db_string("pi.string.create.name").unwrap();
    let probe = db_string("pi.string.create.unique-1").unwrap();
    let props = property_map([(name.clone(), Value::String(probe.clone()))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 5).unwrap();

    assert!(rows(&indexes, label, name, &Value::String(probe)).contains(5));
}

#[test]
fn apply_node_update_moves_bool_index_key() {
    let label = db_string("pi.bool.update.label").unwrap();
    let active = db_string("pi.bool.update.active").unwrap();
    let old_props = property_map([(active.clone(), Value::Bool(false))]);
    let new_props = property_map([(active.clone(), Value::Bool(true))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), active.clone()), entry(TypedIndexKind::Bool));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        9,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        9,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), active.clone(), &Value::Bool(false)).is_empty());
    assert!(rows(&indexes, label, active, &Value::Bool(true)).contains(9));
}

#[test]
fn apply_node_update_moves_u64_index_key() {
    let label = db_string("pi.u64.update.label").unwrap();
    let count = db_string("pi.u64.update.count").unwrap();
    let old_props = property_map([(count.clone(), Value::Uint(7))]);
    let new_props = property_map([(count.clone(), Value::Uint(u64::MAX))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), count.clone()), entry(TypedIndexKind::U64));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        10,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        10,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), count.clone(), &Value::Uint(7)).is_empty());
    assert!(rows(&indexes, label, count, &Value::Uint(u64::MAX)).contains(10));
}

#[test]
fn apply_node_update_moves_exact_numeric_index_keys() {
    let label = db_string("pi.exact.update.label").unwrap();
    let signed = db_string("pi.exact.update.signed").unwrap();
    let unsigned = db_string("pi.exact.update.unsigned").unwrap();
    let amount = db_string("pi.exact.update.amount").unwrap();
    let old_props = property_map([
        (signed.clone(), Value::Int128(i128::MIN + 1)),
        (unsigned.clone(), Value::Uint128(u64::MAX as u128 + 1)),
        (amount.clone(), Value::Decimal(decimal("1.25"))),
    ]);
    let new_props = property_map([
        (signed.clone(), Value::Int128(i128::MAX - 1)),
        (unsigned.clone(), Value::Uint128(u128::MAX - 1)),
        (amount.clone(), Value::Decimal(decimal("2.50"))),
    ]);
    let labels = LabelSet::single(label.clone());
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), signed.clone()), entry(TypedIndexKind::I128));
    indexes.insert(
        (label.clone(), unsigned.clone()),
        entry(TypedIndexKind::U128),
    );
    indexes.insert(
        (label.clone(), amount.clone()),
        entry(TypedIndexKind::Decimal),
    );
    apply_node_create(&mut indexes, &labels, &old_props, 11).unwrap();

    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 11).unwrap();

    assert!(
        rows(
            &indexes,
            label.clone(),
            signed.clone(),
            &Value::Int128(i128::MIN + 1)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            signed,
            &Value::Int128(i128::MAX - 1)
        )
        .contains(11)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            unsigned.clone(),
            &Value::Uint128(u64::MAX as u128 + 1)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            unsigned,
            &Value::Uint128(u128::MAX - 1)
        )
        .contains(11)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            amount.clone(),
            &Value::Decimal(decimal("1.25"))
        )
        .is_empty()
    );
    assert!(rows(&indexes, label, amount, &Value::Decimal(decimal("2.50"))).contains(11));
}

#[test]
fn apply_node_update_moves_temporal_time_index_keys() {
    let label = db_string("pi.temporal.update.label").unwrap();
    let zoned_dt = db_string("pi.temporal.update.zoned-dt").unwrap();
    let local_time = db_string("pi.temporal.update.local-time").unwrap();
    let zoned_time = db_string("pi.temporal.update.zoned-time").unwrap();
    let duration = db_string("pi.temporal.update.duration").unwrap();
    let old_zdt = zoned("2026-05-07T09:00:00-04[America/New_York]");
    let new_zdt = zoned("2026-05-07T12:00:00-04[America/New_York]");
    let old_zt = zoned("2026-05-07T09:30:00-04[America/New_York]");
    let new_zt = zoned("2026-05-07T12:30:00-04[America/New_York]");
    let old_lt = "09:30:00".parse().unwrap();
    let new_lt = "12:30:00".parse().unwrap();
    let old_duration = Value::Duration(Box::new("PT1H".parse().unwrap()));
    let new_duration = Value::Duration(Box::new("PT2H".parse().unwrap()));
    let old_props = property_map([
        (
            zoned_dt.clone(),
            Value::ZonedDateTime(Box::new(old_zdt.clone())),
        ),
        (local_time.clone(), Value::LocalTime(old_lt)),
        (
            zoned_time.clone(),
            Value::ZonedTime(Box::new(old_zt.clone())),
        ),
        (duration.clone(), old_duration.clone()),
    ]);
    let new_props = property_map([
        (
            zoned_dt.clone(),
            Value::ZonedDateTime(Box::new(new_zdt.clone())),
        ),
        (local_time.clone(), Value::LocalTime(new_lt)),
        (
            zoned_time.clone(),
            Value::ZonedTime(Box::new(new_zt.clone())),
        ),
        (duration.clone(), new_duration.clone()),
    ]);
    let labels = LabelSet::single(label.clone());
    let mut indexes = PropertyIndexMap::default();
    indexes.insert(
        (label.clone(), zoned_dt.clone()),
        entry(TypedIndexKind::ZonedDateTime),
    );
    indexes.insert(
        (label.clone(), local_time.clone()),
        entry(TypedIndexKind::LocalTime),
    );
    indexes.insert(
        (label.clone(), zoned_time.clone()),
        entry(TypedIndexKind::ZonedTime),
    );
    indexes.insert(
        (label.clone(), duration.clone()),
        entry(TypedIndexKind::Duration),
    );
    apply_node_create(&mut indexes, &labels, &old_props, 12).unwrap();

    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 12).unwrap();

    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_dt.clone(),
            &Value::ZonedDateTime(Box::new(old_zdt))
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_dt,
            &Value::ZonedDateTime(Box::new(new_zdt))
        )
        .contains(12)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            local_time.clone(),
            &Value::LocalTime(old_lt)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            local_time,
            &Value::LocalTime(new_lt)
        )
        .contains(12)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_time.clone(),
            &Value::ZonedTime(Box::new(old_zt))
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_time,
            &Value::ZonedTime(Box::new(new_zt))
        )
        .contains(12)
    );
    assert!(rows(&indexes, label.clone(), duration.clone(), &old_duration).is_empty());
    assert!(rows(&indexes, label, duration, &new_duration).contains(12));
}

#[test]
fn apply_node_update_moves_string_index_key() {
    // SET against an INDEXED column moves the row from the previous key to
    // the new one.
    let label = db_string("pi.string.update.label").unwrap();
    let name = db_string("pi.string.update.name").unwrap();
    let old = db_string("pi.string.update.old").unwrap();
    let new_key = db_string("pi.string.update.new-unique").unwrap();
    let old_props = property_map([(name.clone(), Value::String(old.clone()))]);
    let new_props = property_map([(name.clone(), Value::String(new_key.clone()))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        9,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        9,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), name.clone(), &Value::String(old)).is_empty());
    assert!(rows(&indexes, label, name, &Value::String(new_key)).contains(9));
}
