# Fix Unused Column False Positive Skill

This skill fixes false positives in bqvalid's `unused_column_in_cte` rule through the following steps.

## Execution Steps

### 1. Verify False Positive and Investigate Root Cause

- Run bqvalid on the SQL file provided by the user
- List all detected unused columns
- Analyze where each column is actually used
- Identify the root cause of false positive (e.g., JOIN conditions, WHERE clauses, GROUP BY clauses, CASE expressions, comments, etc.)

### 2. Create Minimal Test Case

- Create minimal SQL that reproduces the false positive
- Save as `/tmp/test_<issue_type>.sql`
- Generalize table and column names (remove company-specific information)
- Keep it simple (10-30 lines)

**Test Case Naming Convention:**
- `test_<specific_issue_description>.sql`
- Examples: `test_case_expression.sql`, `test_comment_between_select_from.sql`

### 3. Fix Implementation

**Target file:**
- `/Users/sasakawa/git/github.com/hirosassa/bqvalid/src/rules/unused_column_in_cte.rs`

**Important notes:**
- Use `eprintln!` for debug output
- Always remove debug output after fixing
- Understand existing function structure before making changes
- Add new helper functions if needed

**Key functions:**
- `mark_columns_used_in_select_expressions`: Mark column references in SELECT expressions
- `mark_columns_used_in_join_and_where`: Mark columns in JOIN/WHERE/GROUP BY clauses
- `mark_columns_used_in_qualify_clauses`: Mark columns in QUALIFY clauses
- `mark_columns_used_by_select_star`: Mark columns used by SELECT *
- `extract_column_references_from_expression`: Extract column references from expressions
- `extract_columns_from_condition`: Extract column references from conditional expressions

### 4. Add Test Case

- Add the minimal test case to `sql/` directory
- Filename: `unused_column_in_cte_<issue_type>.sql`
- Describe expected behavior in comments within the test case
- Add unit tests if necessary

### 5. Verify Fix

```bash
# Verify with minimal test case
cargo run -- /tmp/test_<issue_type>.sql

# Verify with original SQL
cargo run -- <original_sql_file_path>

# Run all tests
cargo test

# Format and lint
cargo fmt --all -- --check
cargo clippy -- -D warnings -W clippy::nursery
```

### 6. Track Progress with TodoWrite Tool

Use TodoWrite tool to track progress at each step:

```json
[
  {"content": "Verify false positive and investigate root cause", "status": "in_progress", "activeForm": "Verifying false positive and investigating root cause"},
  {"content": "Create minimal test case", "status": "pending", "activeForm": "Creating minimal test case"},
  {"content": "Fix implementation", "status": "pending", "activeForm": "Fixing implementation"},
  {"content": "Add test case", "status": "pending", "activeForm": "Adding test case"},
  {"content": "Verify fix", "status": "pending", "activeForm": "Verifying fix"},
  {"content": "Format and lint", "status": "pending", "activeForm": "Formatting and linting"}
]
```

## Common Fix Patterns

### Pattern 1: Columns in JOIN/WHERE/GROUP BY clauses detected as unused

**Cause:** JOIN conditions in final SELECT are not processed

**Fix:** Process final SELECT in `mark_columns_used_in_join_and_where` function

### Pattern 2: Columns in CASE expressions detected as unused

**Cause:** Comments exist before/after SELECT expressions, FROM clause not found

**Fix:** Call `next_named_sibling()` in a loop to skip comments and find `from_clause`

### Pattern 3: Columns expanded by SELECT * detected as unused

**Cause:** Columns expanded by SELECT * are treated as new definitions

**Fix:** Add `mark_columns_used_by_select_star` function to mark all columns from source table

## Important Notes

1. **Always run tests**: Verify all 27 tests pass after fixing
2. **Remove debug output**: Always remove debug `eprintln!` statements before commit
3. **Minimal test case**: Create minimal SQL that reproduces the issue (don't use large SQL as-is)
4. **Generalize**: Replace company-specific information (table names, column names) with generic names
5. **Comments**: Don't add Japanese comments in implementation (English only)

## Output Format

After executing the skill, report the following information to the user:

1. **Detected false positives**: Which columns were false positives
2. **Root cause**: Why the false positive occurred
3. **Fix details**: Which functions were modified and how
4. **Test results**: Whether all tests passed
5. **Created files**: Path to the added minimal test case
