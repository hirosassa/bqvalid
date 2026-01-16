# Pre-Refactoring Baseline Measurements

Measurement Date: 2026-01-14

## Execution Speed Benchmark (Criterion)

### unused_column_in_cte/check (rule check only on parsed AST)

| Test Case | Execution Time | Unused Columns |
|-----------|----------------|----------------|
| small     | 146.83 µs      | 5              |
| medium    | 309.81 µs      | 37             |
| large     | 1.4395 ms      | 97             |

### parse_and_check (parsing + rule check)

| Test Case | Execution Time |
|-----------|----------------|
| small     | 176.97 µs      |
| medium    | 372.62 µs      |
| large     | 1.7143 ms      |

## Memory Profiling (dhat)

Total across all test cases (small + medium + large):

| Metric                 | Value                        |
|------------------------|------------------------------|
| Total bytes allocated  | 498,123 bytes (approx 486 KB) |
| Max bytes at t-gmax    | 82,189 bytes (approx 80 KB)   |
| Total blocks allocated | 8,380 blocks                 |

## Analysis

### Execution Time Trends

- small to medium: approx 2.1x (CTEs increase from 3 to 10, roughly 3.3x, so slightly better than linear)
- medium to large: approx 4.6x (CTEs increase from 10 to 20, 2x, so worse than linear)
- At large complexity, the impact of multiple AST traversals becomes significant

### Memory Usage Trends

- Peak memory usage is approximately 80KB
- Total memory allocation is approximately 486KB
- Block count of 8,380 indicates frequent small allocations

### Estimated Performance Bottlenecks

The current code likely has the following issues:

1. Multiple AST traversals: Approximately 9 full traversals affect execution time
2. Frequent memory allocations: 8,380 blocks is high (String clones, etc.)
3. Clone overhead: cte_columns cloned 6 times
4. Non-linear complexity: Significant performance degradation at large scale

## Expected Values After Refactoring

### Execution Speed

With single-pass traversal using visitor pattern:

| Test Case | Current   | Target (5-10x) | Target (optimized) |
|-----------|-----------|----------------|--------------------|
| small     | 146.83 µs | 15-30 µs       | 10-20 µs           |
| medium    | 309.81 µs | 31-62 µs       | 20-40 µs           |
| large     | 1.4395 ms | 144-288 µs     | 100-200 µs         |

### Memory Usage

After clone reduction and String interning:

| Metric          | Current | Target (50-80% reduction) |
|-----------------|---------|---------------------------|
| Total allocated | 486 KB  | 97-243 KB                 |
| Max at t-gmax   | 80 KB   | 16-40 KB                  |
| Blocks          | 8,380   | 1,000-4,000               |

## Reproducing Measurements

```bash
# Execution speed benchmark
cargo bench --bench unused_column_in_cte -- --save-baseline before-refactor

# Memory profile
cargo bench --bench memory_profile
cp dhat-heap.json dhat-heap-before-refactor.json
```

## Next Steps

1. Implement refactoring with visitor pattern
2. Run same benchmarks after implementation
3. Compare with before-refactor and confirm improvements
4. Consider additional optimizations if targets are not met

## Notes

- Test environment: macOS Darwin 24.5.0
- Rust version: (check with cargo --version)
- CPU: (system dependent)
- Measurements taken after 3 warmup runs, with 100 sample statistics
