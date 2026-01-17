use tree_sitter::Node;

use super::context::AnalysisContext;

/// Visitor trait for processing AST nodes
pub trait NodeVisitor {
    /// Visit a node and potentially update the analysis context
    fn visit(&self, node: &Node, context: &mut AnalysisContext);
}
