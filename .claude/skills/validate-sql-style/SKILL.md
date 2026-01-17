# Validate SQL Style Skill

This skill validates SQL files in the repository against the project's coding conventions.

## SQL Coding Conventions

### 1. Case Sensitivity
- **All SQL keywords and identifiers must be lowercase**
- Includes: `select`, `from`, `where`, `with`, `as`, `join`, `on`, etc.
- Column names, table names, and aliases must also be lowercase
- Exception: String literals can use any case (e.g., `'Jan'`, `'Feb'`)

### 2. Organization-Specific Terms
- **No organization-specific terms should be present**
- Examples to avoid:
  - Company names (e.g., `acme_`, `mycompany_`)
  - Project-specific prefixes (e.g., `proj_`, `app_`)
  - Department names (e.g., `sales_`, `marketing_`)
- Use generic names instead:
  - Tables: `table1`, `table2`, `source_table`, `base_table`, `users`, `orders`
  - Columns: `column1`, `column2`, `id`, `name`, `value`, `category`
  - CTEs: `data1`, `data2`, `filtered_data`, `aggregated_data`

### 3. Indentation
- **Use 2 spaces for indentation**
- Do not use tabs
- Consistent indentation level for nested structures

### 4. File Format
- Files should end with a single newline character
- Use Unix-style line endings (LF, not CRLF)

### 5. Comments
- Comments should be in English
- Use `--` for single-line comments
- Place comments above the code they describe

## Execution Steps

### 1. Read SQL File

Read the SQL file that needs to be validated.

### 2. Check Case Sensitivity

Scan the SQL file for uppercase SQL keywords or identifiers:
- Look for uppercase keywords: `SELECT`, `FROM`, `WHERE`, `WITH`, `AS`, `JOIN`, `ON`, etc.
- Look for uppercase table names or column names
- Ignore string literals (content within single or double quotes)

Report violations with line numbers.

### 3. Check for Organization-Specific Terms

Scan for common patterns that suggest organization-specific naming:
- Patterns to check:
  - Company/project prefixes: `[a-z]+_[a-z]+_table`, `[a-z]+_db`, etc.
  - Suspiciously specific names (not generic like `table1`, `users`, `orders`)
  - Check against a list of generic table names:
    - `table1`, `table2`, `table3`
    - `source_table`, `base_table`, `target_table`
    - `users`, `orders`, `products`, `customers`
    - Common test data names
- For ambiguous cases, ask the user if the name seems organization-specific

Report potential violations with line numbers and ask for user confirmation.

### 4. Check Indentation

Verify that the file uses 2-space indentation:
- Count leading spaces on each line
- Ensure indentation increases/decreases by 2 spaces
- Report lines with incorrect indentation (1 space, 3 spaces, 4 spaces, tabs, etc.)

Report violations with line numbers.

### 5. Check File Format

- Verify the file ends with a newline
- Check for consistent line endings

Report violations if found.

### 6. Generate Report

Create a summary report:
```
SQL Style Validation Report for: <filename>

✓ Passed Checks:
- [List of passed checks]

✗ Failed Checks:
- [List of failed checks with line numbers and details]

Suggestions:
- [Specific suggestions for fixing each violation]
```

## Example Validation

### Good Example
```sql
-- Test case: Simple CTE usage
with data1 as (
  select
    column1,
    column2
  from
    table1
)
select
  *
from
  data1
```

### Bad Examples

**Uppercase keywords:**
```sql
SELECT column1 FROM table1  -- FAIL: Keywords should be lowercase
```

**Organization-specific names:**
```sql
select * from acme_customer_data  -- FAIL: Contains company name
```

**Wrong indentation:**
```sql
with data1 as (
 select  -- FAIL: 1 space instead of 2
    column1
  from
    table1
)
```

## Output Format

After validation, provide:
1. **Summary**: Number of violations found
2. **Details**: Specific violations with line numbers
3. **Suggestions**: How to fix each violation
4. **Status**: PASS or FAIL

## Usage

To use this skill, provide the path to a SQL file:

```
Please validate the SQL style for sql/my_query.sql
```

Or validate all SQL files in the sql/ directory:

```
Please validate the SQL style for all files in sql/
```
