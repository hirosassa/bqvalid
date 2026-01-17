# bqvalid Skills

This directory contains documentation and workflow guides to assist with bqvalid development.

## Available Guides

### fix-unused-column-false-positive

A workflow guide for fixing false positives in the unused column detection rule.

**Usage:**

Provide SQL with false positives and request as follows:

```
This SQL has false positives. Please fix it following the fix-unused-column-false-positive guide.

<Path to SQL file or its content>
```

**Execution:**

1. Verify false positive and investigate root cause
2. Create minimal test case
3. Fix implementation
4. Add test case
5. Verify fix
6. Format and lint

**Example:**

```
/fix-unused-column-false-positive /tmp/customer_report.sql
```

The skill automatically executes the following steps:

1. Run SQL to detect unused columns
2. Identify root cause of false positive (JOIN conditions, WHERE clauses, CASE expressions, etc.)
3. Create minimal test case (10-30 lines)
4. Fix `src/rules/unused_column_in_cte.rs`
5. Add test case to `sql/` directory
6. Run all tests (27 tests)
7. Run cargo fmt and cargo clippy

**Output:**

- List of detected false positives
- Root cause of false positives
- Description of fixes
- Test results
- Path to created test case

### validate-sql-style

A workflow guide for validating SQL files against the project's coding conventions.

**Usage:**

Validate a single SQL file:

```
Please validate the SQL style for sql/my_query.sql
```

Or validate all SQL files:

```
Please validate the SQL style for all files in sql/
```

**Coding Conventions:**

1. All SQL keywords and identifiers must be lowercase
2. No organization-specific terms (use generic names like table1, users, orders)
3. Use 2 spaces for indentation
4. Files should end with a newline
5. Comments should be in English

**Execution:**

1. Read SQL file(s)
2. Check case sensitivity (no uppercase keywords)
3. Check for organization-specific terms
4. Verify 2-space indentation
5. Check file format
6. Generate validation report

**Output:**

- Summary of passed/failed checks
- Detailed violations with line numbers
- Suggestions for fixing violations
- Overall status: PASS or FAIL

## Developing Skills

When adding a new skill, create a Markdown file in the following format:

```markdown
---
name: skill-name
description: Brief description of the skill
---

# Skill Title

Detailed execution steps in Markdown format
```

Name the file `<skill-name>.md`.
