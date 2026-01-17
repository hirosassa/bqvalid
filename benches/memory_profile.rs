// Memory profiling benchmark using dhat
// Run with: cargo bench --bench memory_profile
// Results will be in dhat-heap.json, visualize at https://nnethercote.github.io/dh_view/dh_view.html

use std::fs;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;

use bqvalid::rules::unused_column_in_cte;

fn parse_sql(sql: &str) -> tree_sitter::Tree {
    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    parser.parse(sql, None).unwrap()
}

fn run_check(name: &str, path: &str) {
    println!("\n=== Running memory profile for: {} ===", name);

    let sql = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));

    let tree = parse_sql(&sql);
    let result = unused_column_in_cte::check(&tree, &sql);

    match result {
        Some(diagnostics) => {
            println!("Found {} unused columns", diagnostics.len());
        }
        None => {
            println!("No unused columns found");
        }
    }
}

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!("Starting memory profiling...");

    run_check("small", "./benches/fixtures/bench_small.sql");
    run_check("medium", "./benches/fixtures/bench_medium.sql");
    run_check("large", "./benches/fixtures/bench_large.sql");

    println!("\n=== Memory profiling complete ===");
    println!("Results saved to dhat-heap.json");
    println!("Visualize at: https://nnethercote.github.io/dh_view/dh_view.html");
}
