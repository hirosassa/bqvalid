# Benchmarks

Benchmarks for measuring the performance of the unused_column_in_cte rule.

## Test Cases

Benchmarks are run on SQL queries of three different sizes:

- small: 3 CTEs, simple structure
- medium: 10 CTEs, moderate complexity
- large: 20 CTEs, high complexity

## How to Run

### 1. Execution Speed Benchmark

Detailed benchmarking using Criterion:

```bash
# Run all benchmarks
cargo bench --bench unused_column_in_cte

# Run specific benchmark only
cargo bench --bench unused_column_in_cte -- check

# Save baseline (before refactoring)
cargo bench --bench unused_column_in_cte -- --save-baseline before

# After refactoring, compare with baseline
cargo bench --bench unused_column_in_cte -- --baseline before
```

Results are saved in `target/criterion/` and an HTML report is generated.

### 2. Memory Profiling

Detailed memory usage analysis using dhat:

```bash
# Run memory profile
cargo bench --bench memory_profile

# Results are saved in dhat-heap.json
# Visualize at:
# https://nnethercote.github.io/dh_view/dh_view.html
```

## Interpreting Benchmark Results

### Criterion (Execution Speed)

```
unused_column_in_cte/check/small
                        time:   [500.00 µs 505.00 µs 510.00 µs]
                        change: [-5.0% +0.5% +6.0%] (p = 0.87 > 0.05)
                        No change in performance detected.
```

- time: Estimated execution time (lower bound, median, upper bound)
- change: Change rate from previous run

### dhat (Memory Usage)

Visualizing dhat-heap.json provides the following information:

- Total bytes allocated: Total memory allocation
- Max bytes allocated: Peak memory usage
- Total blocks allocated: Number of memory block allocations
- At t-gmax: Details at peak memory usage time

## Recording Baseline Before Refactoring

Always save baseline before refactoring:

```bash
# Execution speed baseline
cargo bench --bench unused_column_in_cte -- --save-baseline before-refactor

# Memory baseline
cargo bench --bench memory_profile
cp dhat-heap.json dhat-heap-before-refactor.json
```

## Performance Goals

Expected improvements after refactoring:

- Execution speed: 5-10x speedup (by reducing multiple AST traversals)
- Memory usage: 50-80% reduction (by reducing unnecessary clones)

## Troubleshooting

### When benchmarks fail

```bash
# First, confirm build succeeds
cargo build --release

# Confirm tests pass
cargo test

# Check benchmark files exist
ls -l benches/fixtures/
```

### When memory profile is not generated

dhat generates dhat-heap.json at runtime. If the file is not generated:

1. Check if the program exits normally
2. Check write permissions
3. Adjust settings with DHAT_OPTIONS environment variable

```bash
DHAT_OPTIONS="--show-top-n=50" cargo bench --bench memory_profile
```
