-- Medium benchmark: 10 CTEs with moderate complexity
WITH users_base AS (
  SELECT
    id,
    name,
    email,
    age,
    country,
    created_at,
    updated_at,
    unused_col1
  FROM `project.dataset.users`
),
orders_base AS (
  SELECT
    order_id,
    user_id,
    product_id,
    quantity,
    amount,
    order_date,
    unused_col1
  FROM `project.dataset.orders`
),
user_orders AS (
  SELECT
    u.id,
    u.name,
    u.email,
    o.order_id,
    o.amount,
    o.created_at as order_date,
    unused_col1
  FROM data1 u
  JOIN `project.dataset.orders` o ON u.id = o.user_id
  WHERE u.created_at > '2024-01-01'
),
order_stats AS (
  SELECT
    user_id,
    COUNT(*) as order_count,
    SUM(amount) as total_amount,
    AVG(amount) as avg_amount,
    MAX(amount) as max_amount,
    MIN(amount) as min_amount,
    unused_stat1,
    unused_stat2
  FROM `project.dataset.orders`
  GROUP BY user_id
),
user_segments AS (
  SELECT
    id,
    name,
    CASE
      WHEN total_amount > 10000 THEN 'premium'
      WHEN total_amount > 5000 THEN 'gold'
      ELSE 'standard'
    END as segment,
    unused_segment_field
  FROM data2
),
final_data AS (
  SELECT
    data3.id,
    data3.name,
    data3.order_count,
    data3.total_amount,
    RANK() OVER (ORDER BY data3.total_amount DESC) as rank,
    unused_final_field
  FROM data3
)
SELECT * FROM data3
