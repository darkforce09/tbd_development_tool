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
    /// A package depends on another, by a path dependency in its manifest. Runs between the two manifest cards.
    Depends,
}

impl EdgeKind {
    /// Number of kinds, for per-kind tables.
    pub const COUNT: usize = 7;
    /// Every kind, in `as usize` order.
    pub const ALL: [EdgeKind; EdgeKind::COUNT] = [
        EdgeKind::Import,
        EdgeKind::Call,
        EdgeKind::TypeUse,
        EdgeKind::Implements,
        EdgeKind::Documentation,
        EdgeKind::Asset,
        EdgeKind::Depends,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Import => "import",
            EdgeKind::Call => "call",
            EdgeKind::TypeUse => "uses type",
            EdgeKind::Implements => "implements",
            EdgeKind::Documentation => "documentation",
            EdgeKind::Asset => "asset",
            EdgeKind::Depends => "depends on",
        }
    }

    /// Code-flow kinds (package dependencies included) decide the left-to-right layout; documentation and asset
    /// wires do not.
    pub fn is_code_flow(self) -> bool {
        !matches!(self, EdgeKind::Documentation | EdgeKind::Asset)
    }
}

/// How sure the map is that a link exists (docs/ROADMAP.md S2). Every link shows its tier; nothing
/// is drawn as certain unless it was resolved exactly.
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
pub enum EvidenceTier {
    /// Resolved exactly: by a compiler, a manifest, or an exact path, tag, route or pointer.
    Proven,
    /// One of a known set of candidates; [`Provenance::candidates`] says how many.
    PossibleSet,
    /// Seen happening: in a run, a trace or an agent's session log.
    Observed,
    /// Matched by name only. Drawn, but never as a fact.
    #[default]
    Unresolved,
}

impl EvidenceTier {
    pub fn label(self) -> &'static str {
        match self {
            EvidenceTier::Proven => "proven",
            EvidenceTier::PossibleSet => "possible",
            EvidenceTier::Observed => "observed",
            EvidenceTier::Unresolved => "unresolved",
        }
    }
}

/// What a link was worked out from.
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
pub enum Basis {
    /// The same name in both places.
    #[default]
    NameMatch,
    /// A documentation link to a file path that exists.
    DocLink,
    /// A dependency declared in a package manifest (Cargo.toml, package.json, ...).
    Manifest,
    /// A source path resolved through modules and imports (`use`, `mod`, `import`).
    PathResolution,
    /// A `@route` or `@contract` tag matched exactly.
    Tag,
    /// An entry in an HTTP route table or a command-line definition.
    RouteTable,
    /// A JSON pointer into a schema or contract file.
    JsonPointer,
    /// A run, a trace or a session log.
    Runtime,
}

/// Where a link comes from and how sure it is.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq, Hash))]
pub struct Provenance {
    pub tier: EvidenceTier,
    pub basis: Basis,
    /// For [`EvidenceTier::PossibleSet`]: how many candidates there were, this one included.
    pub candidates: u16,
}

impl Provenance {
    /// Resolved exactly, from `basis`.
    pub fn proven(basis: Basis) -> Self {
        Self { tier: EvidenceTier::Proven, basis, candidates: 1 }
    }

    /// One of `candidates` possible targets, from `basis`.
    pub fn possible(basis: Basis, candidates: u16) -> Self {
        Self { tier: EvidenceTier::PossibleSet, basis, candidates }
    }

    /// Seen at run time.
    pub fn observed() -> Self {
        Self { tier: EvidenceTier::Observed, basis: Basis::Runtime, candidates: 1 }
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
    /// Where the link comes from and how sure it is. Name matches unless set otherwise.
    pub provenance: Provenance,
}
