#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "benchmark code"
)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::fs;
use std::hint::black_box;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;

// Import the rule check function
use bqvalid::ast::Ast;
use bqvalid::rules::unused_column_in_cte;

fn parse_sql(sql: &str) -> Ast {
    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    let tree = parser.parse(sql, None).unwrap();
    Ast::from_tree_sitter(&tree)
}

fn bench_unused_column_check(c: &mut Criterion) {
    let test_cases = vec![
        ("small", "./benches/fixtures/bench_small.sql"),
        ("medium", "./benches/fixtures/bench_medium.sql"),
        ("large", "./benches/fixtures/bench_large.sql"),
    ];

    let mut group = c.benchmark_group("unused_column_in_cte");

    for (name, path) in test_cases {
        let sql = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));
        let ast = parse_sql(&sql);

        group.bench_with_input(
            BenchmarkId::new("check", name),
            &(&ast, &sql),
            |b, (ast, sql)| {
                b.iter(|| {
                    let result = unused_column_in_cte::check(black_box(ast), black_box(sql));
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

fn bench_parse_and_check(c: &mut Criterion) {
    let test_cases = vec![
        ("small", "./benches/fixtures/bench_small.sql"),
        ("medium", "./benches/fixtures/bench_medium.sql"),
        ("large", "./benches/fixtures/bench_large.sql"),
    ];

    let mut group = c.benchmark_group("parse_and_check");

    for (name, path) in test_cases {
        let sql = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));

        group.bench_with_input(BenchmarkId::new("full", name), &sql, |b, sql| {
            b.iter(|| {
                let ast = parse_sql(black_box(sql));
                let result = unused_column_in_cte::check(black_box(&ast), black_box(sql));
                black_box(result);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_unused_column_check, bench_parse_and_check);
criterion_main!(benches);
