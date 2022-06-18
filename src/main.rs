use std::process::ExitCode;
use std::{io, io::Read, result::Result};
use thiserror::Error;
use tree_sitter::{Node, Parser};
use tree_sitter_sql_bigquery::language;
use tree_sitter_traversal::{traverse, Order};

fn main() -> ExitCode {
    let mut parser = Parser::new();
    parser.set_language(language()).unwrap();

    let mut sql = String::new();
    let _ = io::stdin().read_to_string(&mut sql);
    let tree = parser.parse(&sql, None).unwrap();

    for node in traverse(tree.walk(), Order::Pre) {
        if node.kind() == "where_clause" {
            if let Err(Error::ComparedWithSubquery(err)) =
                compared_with_subquery_in_binary_expression(node, &sql)
            {
                eprintln!("{}", Error::ComparedWithSubquery(err));
                return ExitCode::FAILURE;
            }
            if let Err(Error::ComparedWithSubquery(err)) =
                compared_with_subquery_in_between_expression(node, &sql)
            {
                eprintln!("{}", Error::ComparedWithSubquery(err));
                return ExitCode::FAILURE;
            }
        }
    }
    print!("{}", sql);
    ExitCode::SUCCESS
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Full scan will cause! Compared _TABLE_SUFFIX with subquery{0}")]
    ComparedWithSubquery(String),
}

fn compared_with_subquery_in_binary_expression(n: Node, src: &str) -> Result<(), Error> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            let mut tc = parent.walk();
            let right_operand = parent.children(&mut tc).last().unwrap();
            if parent.kind() == "binary_expression"
                && right_operand.kind() == "select_subexpression"
            {
                let rg = right_operand.range();
                let error_part = &src[rg.start_byte..rg.end_byte];
                return Err(Error::ComparedWithSubquery(format!(
                    "\nstart at: line {start}\nend at: line {end}\nexpression:\n{part}",
                    start = rg.start_point.row,
                    end = rg.end_point.row,
                    part = error_part,
                )));
            }
        }
    }
    Ok(())
}

fn compared_with_subquery_in_between_expression(n: Node, src: &str) -> Result<(), Error> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            if parent.kind() == "between_operator" {
                let mut tc = parent.walk();
                for c in parent.children(&mut tc) {
                    if (c.kind() == "between_from" || c.kind() == "between_to")
                        && c.child(0).unwrap().kind() == "select_subexpression"
                    {
                        let rg = c.child(0).unwrap().range();
                        let error_part = &src[rg.start_byte..rg.end_byte];
                        return Err(Error::ComparedWithSubquery(format!(
                            "\nstart at: line {start}\nend at: line {end}\nexpression:\n{part}",
                            start = rg.start_point.row,
                            end = rg.end_point.row,
                            part = error_part,
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn valid() {
        let mut parser = Parser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/valid.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert_eq!(
                    compared_with_subquery_in_binary_expression(node, &sql).is_err(),
                    false
                );
                assert_eq!(
                    compared_with_subquery_in_between_expression(node, &sql).is_err(),
                    false
                );
            }
        }
    }

    #[test]
    fn binary_op() {
        let mut parser = Parser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_binary_op.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_binary_expression(node, &sql).is_err());
            }
        }
    }

    #[test]
    fn between_from() {
        let mut parser = Parser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_from.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_err());
            }
        }
    }

    #[test]
    fn between_to() {
        let mut parser = Parser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_to.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_err());
            }
        }
    }
}
