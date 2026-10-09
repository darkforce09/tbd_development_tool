use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use super::types::{EdgeId, NodeId, PortId};

/// What a wire means. Defined in docs/VISUAL_LANGUAGE.md, which also sets each kind's colour.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq, Hash))]
pub enum EdgeKind {
    /// A file or module imports or includes another.
    #[default]
    Import,
    /// A function calls another.
    Call,
    /// Code uses a type defined elsewhere.
    TypeUse,
    /// A type implements or inherits a trait, interface or base class.
    Implements,
    /// Documentation links to code. Ends at the target's documentation port.
    Documentation,
    /// Code or data references an asset.
    Asset,
}

impl EdgeKind {
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Import => "import",
            EdgeKind::Call => "call",
            EdgeKind::TypeUse => "uses type",
            EdgeKind::Implements => "implements",
            EdgeKind::Documentation => "documentation",
            EdgeKind::Asset => "asset",
        }
    }

    /// Code-flow kinds decide the left-to-right layout; documentation and asset wires do not.
    pub fn is_code_flow(self) -> bool {
        !matches!(self, EdgeKind::Documentation | EdgeKind::Asset)
    }
}

/// A wire from a provider's output port to a consumer's input port.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq, Hash))]
pub struct Edge {
    pub id: EdgeId,
    pub from_node: NodeId,
    pub from_port: PortId,
    pub to_node: NodeId,
    pub to_port: PortId,
    pub kind: EdgeKind,
    pub label: Option<String>,
    pub step_number: Option<usize>,
    pub source_line: Option<usize>,
}
