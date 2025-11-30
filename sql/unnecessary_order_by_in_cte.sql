-- Unnecessary ORDER BY in CTE
WITH sorted_data AS (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id  -- This ORDER BY is ignored
)
SELECT
  *
FROM
  sorted_data
