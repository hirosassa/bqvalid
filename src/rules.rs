pub mod compare_table_suffix_with_subquery;
pub mod helpers;
pub mod invalid_group_by;
pub mod rule;
pub mod unnecessary_order_by;
pub mod unused_column_in_cte;
pub mod use_current_date;

pub use rule::{Rule, all_rules, known_rule_ids, run_rules, run_rules_ignoring};
