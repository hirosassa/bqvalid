use crate::ast::NodeRef;

use super::context::AnalysisContext;

/// Visitor trait for processing AST nodes
pub trait NodeVisitor {
    /// Visit a node and potentially update the analysis context
    fn visit(&self, node: NodeRef<'_>, context: &mut AnalysisContext);
}
