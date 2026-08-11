// Memory profiling benchmark using dhat
// Run with: cargo bench --bench memory_profile
// Results will be in dhat-heap.json, visualize at https://nnethercote.github.io/dh_view/dh_view.html
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "benchmark code"
)]

use googlesql::Module;
use std::fs;

use bqvalid::ast::Ast;
use bqvalid::rules::unused_column_in_cte;

fn parse_sql(module: &mut Module, sql: &str) -> Ast {
    Ast::from_googlesql(module, sql).expect("googlesql parses the sql")
}

fn run_check(module: &mut Module, name: &str, path: &str) {
    println!("\n=== Running memory profile for: {} ===", name);

    let sql = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));

    let ast = parse_sql(module, &sql);
    let diagnostics = unused_column_in_cte::check(&ast, &sql);

    if diagnostics.is_empty() {
        println!("No unused columns found");
    } else {
        println!("Found {} unused columns", diagnostics.len());
    }
}

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!("Starting memory profiling...");

    let mut module = bqvalid::build_module().expect("googlesql module builds");
    run_check(&mut module, "small", "./benches/fixtures/bench_small.sql");
    run_check(&mut module, "medium", "./benches/fixtures/bench_medium.sql");
    run_check(&mut module, "large", "./benches/fixtures/bench_large.sql");

    println!("\n=== Memory profiling complete ===");
    println!("Results saved to dhat-heap.json");
    println!("Visualize at: https://nnethercote.github.io/dh_view/dh_view.html");
}
