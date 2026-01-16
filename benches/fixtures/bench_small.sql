-- Small benchmark: 3 CTEs with simple structure
WITH data1 AS (
  SELECT
    id,
    name,
    email,
    created_at,
    unused_field1
  FROM `project.dataset.users`
),
data2 AS (
  SELECT
    data1.id,
    data1.name,
    COUNT(*) as order_count,
    SUM(amount) as total_amount,
    unused_field2
  FROM data1
  JOIN `project.dataset.orders` orders ON data1.id = orders.user_id
  GROUP BY data1.id, data1.name
),
data3 AS (
  SELECT
    data2.id,
    data2.name,
    data2.order_count,
    data2.total_amount,
    unused_field3
  FROM data2
  WHERE data2.total_amount > 1000
)
SELECT
  id,
  name,
  order_count,
  total_amount
FROM data3
ORDER BY total_amount DESC
