#![allow(missing_docs)]
//! Focused semantic execution-context and result-conversion costs.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::{hint::black_box, sync::Arc};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use selene_core::{Value, db_string};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, ExecutionContext,
    ExecutionOutcome, ExecutionStack, Record, StatementOutput,
};

fn schema() -> BindingTableSchema {
    BindingTableSchema {
        columns: vec![
            BindingTableColumn {
                name: Some(db_string("id").unwrap()),
                hidden: None,
                ty: AnalyzedType::Resolved(selene_gql::GqlType::Integer),
            },
            BindingTableColumn {
                name: Some(db_string("active").unwrap()),
                hidden: None,
                ty: AnalyzedType::Resolved(selene_gql::GqlType::Boolean),
            },
        ],
    }
}

fn small_table() -> BindingTable {
    BindingTable::new(
        schema(),
        vec![
            Binding::new([Value::Int(1), Value::Bool(true)]),
            Binding::new([Value::Int(2), Value::Bool(false)]),
        ],
    )
}

fn bench_execution_context(c: &mut Criterion) {
    let mut group = c.benchmark_group("execution_context");
    group.bench_function("root_create", |b| {
        b.iter(|| black_box(ExecutionContext::root()));
    });
    group.bench_function("child_create_cleanup", |b| {
        b.iter(|| {
            let mut stack = ExecutionStack::new();
            let child = stack.push_child();
            black_box(child.context());
        });
    });

    let record = Record::new([(db_string("seed").unwrap(), Value::Int(1))]).unwrap();
    let amended_name = db_string("amended").unwrap();
    group.bench_function("record_amend", |b| {
        b.iter(|| {
            black_box(&record)
                .amend([(black_box(amended_name.clone()), Value::Int(2))])
                .unwrap()
        });
    });
    group.bench_function("small_table_construct", |b| {
        b.iter(|| black_box(small_table()));
    });

    let table = Arc::new(small_table());
    group.bench_function("result_conversion_allocating", |b| {
        b.iter_batched(
            || table.as_ref().clone(),
            |table| {
                black_box(ExecutionOutcome::from_statement(
                    StatementOutput::Rows(table),
                    Vec::new(),
                ))
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group! {
    name = execution_context;
    config = common::criterion_config();
    targets = bench_execution_context
}
criterion_main!(execution_context);
