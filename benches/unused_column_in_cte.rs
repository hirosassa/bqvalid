use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::fs;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;

// Import the rule check function
use bqvalid::rules::unused_column_in_cte;

fn parse_sql(sql: &str) -> tree_sitter::Tree {
    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    parser.parse(sql, None).unwrap()
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
        let tree = parse_sql(&sql);

        group.bench_with_input(
            BenchmarkId::new("check", name),
            &(&tree, &sql),
            |b, (tree, sql)| {
                b.iter(|| {
                    let result = unused_column_in_cte::check(black_box(tree), black_box(sql));
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
                let tree = parse_sql(black_box(sql));
                let result = unused_column_in_cte::check(black_box(&tree), black_box(sql));
                black_box(result);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_unused_column_check, bench_parse_and_check);
criterion_main!(benches);
