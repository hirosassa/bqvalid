# Contributing

We welcome code contributions for new features and bug fixes!

If you want to add new linting rules, use the following steps:

1. Check the [issues page](https://github.com/hirosassa/bqvalid/issues) on GitHub to see if the task you want to complete is listed there.
1. Create an issue branch for your local work.
1. Add your code in `src/rules/` and implement `pub fn check(tree &Tree, sql: &str)` function in it.
1. Call your new rules from `analyse_sql` function in `src/main.rs`.
1. Write unit tests for your code and make sure everything is still working.
1. Submit a pull request to the main branch of this repository.

