# Contributing

We welcome code contributions for new features and bug fixes!

If you want to add new linting rules, use the following steps:

1. Check the [issues page](https://github.com/hirosassa/bqvalid/issues) on GitHub to see if the task you want to complete is listed there.
1. Create an issue branch for your local work.
1. Add your code in `src/rules/` and implement the `Rule` trait (defined in `src/rules/rule.rs`) for it:
   - `id` returns a stable, unique identifier for the rule (used in machine-readable output).
   - Node-driven rules override `check_node`, which is called once per node during the shared pre-order traversal.
   - Rules that need cross-node analysis override `check_tree` and walk the tree themselves.
   - Each `Diagnostic` you emit carries a `Severity` (`Error` for queries BigQuery would reject, `Warning` for performance/maintainability problems).
1. Register your rule by adding one entry to `all_rules()` in `src/rules/rule.rs`. This is the single place rules are wired in; you do not need to touch the analysis loop or `src/main.rs`.
1. Write unit tests for your code and make sure everything is still working.
1. Submit a pull request to the main branch of this repository. For complex implementations, add performance benchmarks in `benches/` and include the results in the pull request description.

