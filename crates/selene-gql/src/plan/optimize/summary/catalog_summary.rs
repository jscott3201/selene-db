//! Catalog operation payload summaries.

use crate::plan::CatalogOp;

pub(super) fn catalog_summary(catalog: &CatalogOp) -> String {
    match catalog {
        CatalogOp::CreateGraph { name, .. } => format!("op=CreateGraph(name={})", name.as_str()),
        CatalogOp::DropGraph { name, .. } => format!("op=DropGraph(name={})", name.as_str()),
        CatalogOp::CreateNodeType {
            label, properties, ..
        } => format!(
            "op=CreateNodeType(label={}, props={})",
            label.as_str(),
            properties.len()
        ),
        CatalogOp::CreateEdgeType {
            label, properties, ..
        } => format!(
            "op=CreateEdgeType(label={}, props={})",
            label.as_str(),
            properties.len()
        ),
        CatalogOp::DropNodeType { label, .. } => {
            format!("op=DropNodeType(label={})", label.as_str())
        }
        CatalogOp::DropEdgeType { label, .. } => {
            format!("op=DropEdgeType(label={})", label.as_str())
        }
        CatalogOp::CreateIndex {
            name,
            label,
            properties,
            ..
        } => format!(
            "op=CreateIndex(name={}, label={}, props={})",
            name.as_str(),
            label.as_str(),
            properties.len()
        ),
        CatalogOp::DropIndex { name, .. } => format!("op=DropIndex(name={})", name.as_str()),
        CatalogOp::ShowNodeTypes(_) => "op=ShowNodeTypes".to_owned(),
        CatalogOp::ShowEdgeTypes(_) => "op=ShowEdgeTypes".to_owned(),
        CatalogOp::ShowIndexes(_) => "op=ShowIndexes".to_owned(),
        CatalogOp::ShowProcedures(_) => "op=ShowProcedures".to_owned(),
    }
}
