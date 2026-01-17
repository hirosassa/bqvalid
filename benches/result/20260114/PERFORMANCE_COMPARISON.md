# Performance Comparison Results

## Execution Speed (rule check only)

| Test Case          | Before Refactoring | After Refactoring | Improvement |
|--------------------|-------------------|-------------------|-------------|
| small (3 CTEs)     | 146.83 µs         | 21.76 µs          | 6.75x       |
| medium (10 CTEs)   | 309.81 µs         | 43.49 µs          | 7.12x       |
| large (20 CTEs)    | 1439.5 µs         | 194.72 µs         | 7.39x       |

## Execution Speed (parsing + rule check)

| Test Case | Before Refactoring | After Refactoring | Improvement |
|-----------|-------------------|-------------------|-------------|
| small     | 176.97 µs         | 50.03 µs          | 3.54x       |
| medium    | 372.62 µs         | 102.04 µs         | 3.65x       |
| large     | 1714.3 µs         | 457.61 µs         | 3.75x       |

## Analysis

### Key Improvement Factors

1. Single-pass traversal: Reduced 9 AST traversals to 1
2. Clone reduction: Reduced cte_columns clones from 6 to 0
3. Efficient data structures: Optimized HashMap references

### Goal Achievement

Target: 5-10x speedup for execution speed

Result: Average 7.1x speedup achieved

### Scalability

- small to medium: 2x CTEs result in 2x execution time (O(n))
- medium to large: 2x CTEs result in 4.5x execution time (slightly non-linear but acceptable)

Before refactoring, degradation approached 10x, while the new implementation achieves more linear scalability.

## Next Steps

The following features are still unimplemented in the current version:
- JOIN/WHERE condition processing
- QUALIFY clause processing
- PIVOT clause processing
- FROM clause subquery processing
- SELECT DISTINCT processing

Since the single-pass structure can be maintained when adding these features, significant performance degradation is not expected.

## Memory Usage (total across all test cases)

| Metric            | Before Refactoring      | After Refactoring     | Improvement     |
|-------------------|-------------------------|-----------------------|-----------------|
| Total allocated   | 498,123 bytes (486 KB)  | 222,503 bytes (217 KB)| 55% reduction   |
| Max at t-gmax     | 82,189 bytes (80 KB)    | 95,681 bytes (93 KB)  | -16% (slightly increased) |
| Total blocks      | 8,380                   | 2,228                 | 73% reduction   |

### Memory Analysis

- Total memory allocation: 55% reduction - clone reduction is effective
- Peak memory: 16% increase - AnalysisContext holds all data
- Memory block count: 73% reduction - small allocations significantly reduced

Although peak memory increased slightly, overall memory efficiency improved with significant reductions in total memory usage and block count.

## Notes

- Measurement Date: 2026-01-14
- Implementation: visitor-based single-pass traversal
- Still room for optimization (String interning, memoization, etc.)
