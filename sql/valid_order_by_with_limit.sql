-- Valid: ORDER BY with LIMIT in CTE
WITH top_users AS (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id
  LIMIT 10
)
SELECT
  *
FROM
  top_users

-- Valid: ORDER BY with LIMIT in subquery
SELECT
  *
FROM (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id
  LIMIT 10
)

-- Valid: ORDER BY in final SELECT
SELECT
  id,
  name
FROM
  table1
ORDER BY id
