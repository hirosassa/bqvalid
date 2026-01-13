with
dim_contract as (
  select distinct -- コメント
    team_id
    , start_date
    , end_date
    , contract_type
    , base_fee
    , account_fee
    , free_account_count
  from `project`.`dataset`.`dim_contract`
  where
    team_id is not null
    and start_date is not null
    and contract_type = '無償提供'
)

select
  team_id,
  start_date,
  end_date
from
  dim_contract
