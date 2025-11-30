-- Unnecessary ORDER BY in subquery
SELECT
  *
FROM (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id  -- This ORDER BY is ignored
)
WHERE
  name = 'test'
