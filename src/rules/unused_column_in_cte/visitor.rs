use tree_sitter::Node;

use super::context::AnalysisContext;

/// Visitor trait for processing AST nodes
pub trait NodeVisitor {
    /// Visit a node and potentially update the analysis context
    fn visit(&self, node: &Node, context: &mut AnalysisContext);
}

/// Composite visitor that runs multiple visitors on each node
pub struct CompositeVisitor {
    visitors: Vec<Box<dyn NodeVisitor>>,
}

impl CompositeVisitor {
    #[allow(dead_code)]
    pub fn new(visitors: Vec<Box<dyn NodeVisitor>>) -> Self {
        Self { visitors }
    }

    /// Visit a node with all registered visitors
    #[allow(dead_code)]
    pub fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        for visitor in &self.visitors {
            visitor.visit(node, context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestVisitor {
        node_kind: &'static str,
    }

    impl NodeVisitor for TestVisitor {
        fn visit(&self, node: &Node, _context: &mut AnalysisContext) {
            // Test visitor just checks node kind
            let _ = node.kind() == self.node_kind;
        }
    }

    #[test]
    fn test_composite_visitor_creation() {
        let visitors: Vec<Box<dyn NodeVisitor>> = vec![
            Box::new(TestVisitor { node_kind: "cte" }),
            Box::new(TestVisitor {
                node_kind: "select",
            }),
        ];

        let composite = CompositeVisitor::new(visitors);
        assert_eq!(composite.visitors.len(), 2);
    }
}
