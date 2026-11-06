pub mod azure_devops;
mod fence;

#[cfg(test)]
mod tests;

use clap::ValueEnum;

use crate::markdown::model::{Block, DiagramBlock, Document};
use crate::source::SourceText;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Flavor {
    Gfm,
    #[value(name = "azure-devops")]
    AzureDevops,
}

pub fn parse(source: SourceText, flavor: Flavor) -> Document {
    let mut document = match flavor {
        Flavor::Gfm => crate::markdown::parse(source),
        Flavor::AzureDevops => azure_devops::parse(source),
    };
    promote_diagrams(&mut document.blocks);
    document
}

fn promote_diagrams(blocks: &mut [Block]) {
    for block in blocks {
        if let Block::Code(code) = block
            && code.language.as_deref() == Some("mermaid")
        {
            *block = Block::Diagram(DiagramBlock::new(
                "mermaid".into(),
                std::mem::take(&mut code.text),
                code.range,
            ));
        }
        for children in block.children_mut() {
            promote_diagrams(children);
        }
    }
}
