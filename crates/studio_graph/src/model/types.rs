use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Compact 32-byte GPU-friendly node instance data aligned for high-bandwidth DDR5/VRAM streaming
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    bytemuck::Pod,
    bytemuck::Zeroable,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(derive(Debug))]
pub struct GpuNodeInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub color_rgba: u32,
    pub border_color_rgba: u32,
    pub archetype_flags: u32,
    pub flags: u32,
}

/// Compact 64-byte GPU-friendly edge curve instance data
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    bytemuck::Pod,
    bytemuck::Zeroable,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(derive(Debug))]
pub struct GpuEdgeInstance {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
    pub p3: [f32; 2],
    pub color_rgba: u32,
    pub width: f32,
    pub anim_phase: f32,
    pub flags: u32,
}

#[derive(
    Debug,
    Clone,
    Copy,
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
#[rkyv(derive(Debug, PartialEq, PartialOrd, Ord, Eq, Hash))]
pub struct NodeId(pub u64);

#[derive(
    Debug,
    Clone,
    Copy,
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
#[rkyv(derive(Debug, PartialEq, PartialOrd, Ord, Eq, Hash))]
pub struct PortId(pub u64);

#[derive(
    Debug,
    Clone,
    Copy,
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
#[rkyv(derive(Debug, PartialEq, PartialOrd, Ord, Eq, Hash))]
pub struct EdgeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq, Hash))]
pub enum DataType {
    Audio,
    Vision,
    Text,
    State,
    Flow,
    Composite,
    RustType(String),
    RustFlow,
    /// A documentation port: documentation that describes the node links in here.
    Documentation,
}

impl DataType {
    pub fn display_name(&self) -> &str {
        match self {
            DataType::Audio => "AudioPacket",
            DataType::Vision => "ArchivedFrame",
            DataType::Text => "SubtitleText",
            DataType::State => "StateStore",
            DataType::Flow => "Trigger",
            DataType::Composite => "CompositeFrame",
            DataType::RustType(name) => name.as_str(),
            DataType::RustFlow => "Flow",
            DataType::Documentation => "Documentation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq, Hash))]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct Port {
    pub id: PortId,
    pub name: String,
    pub data_type: DataType,
    pub direction: PortDirection,
}

#[repr(usize)]
#[derive(
    Debug,
    Clone,
    Copy,
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
#[rkyv(derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash))]
pub enum NodeArchetype {
    Ingress = 0,
    Compute = 1,
    State = 2,
    Egress = 3,
    Module = 4,
    Function = 5,
    Struct = 6,
    Enum = 7,
    Trait = 8,
    File = 9,
    /// A link inside a documentation file.
    Link = 10,
}

impl NodeArchetype {
    /// Number of variants, for per-archetype tables.
    pub const COUNT: usize = 11;

    #[inline]
    pub fn index(&self) -> usize {
        *self as usize
    }

    pub fn label(&self) -> &'static str {
        match self {
            NodeArchetype::Ingress => "INGRESS",
            NodeArchetype::Compute => "COMPUTE",
            NodeArchetype::State => "STATE",
            NodeArchetype::Egress => "EGRESS",
            NodeArchetype::Module => "MODULE",
            NodeArchetype::Function => "FUNCTION",
            NodeArchetype::Struct => "STRUCT",
            NodeArchetype::Enum => "ENUM",
            NodeArchetype::Trait => "TRAIT",
            NodeArchetype::File => "FILE",
            NodeArchetype::Link => "LINK",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct FileMemberNode {
    pub id: String,
    pub name: String,
    pub archetype: NodeArchetype,
    pub visibility: String,
    pub signature: String,
    pub line_number: usize,
    pub source_code: String,
    pub doc_comment: Option<String>,
    pub in_port_id: Option<PortId>,
    pub out_port_id: Option<PortId>,
}

impl FileMemberNode {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        archetype: NodeArchetype,
        visibility: impl Into<String>,
        signature: impl Into<String>,
        line_number: usize,
        source_code: impl Into<String>,
        doc_comment: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            archetype,
            visibility: visibility.into(),
            signature: signature.into(),
            line_number,
            source_code: source_code.into(),
            doc_comment,
            in_port_id: None,
            out_port_id: None,
        }
    }

    pub fn with_ports(mut self, in_port: Option<PortId>, out_port: Option<PortId>) -> Self {
        self.in_port_id = in_port;
        self.out_port_id = out_port;
        self
    }
}

/// What a file card holds, so the UI knows whether it can be parsed, previewed or edited.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum FileContent {
    /// Text that was parsed (or a non-file node).
    #[default]
    Code,
    Binary,
    Image,
    /// Too large to parse; still listed.
    TooLarge,
    /// Symbolic link; holds the link target.
    Symlink(String),
    /// Could not be read; holds the error.
    Unreadable(String),
}

impl FileContent {
    /// Short reason shown on cards for anything that is not parsed code.
    pub fn label(&self) -> Option<String> {
        match self {
            FileContent::Code => None,
            FileContent::Binary => Some("binary".to_string()),
            FileContent::Image => Some("image".to_string()),
            FileContent::TooLarge => Some("too large to parse".to_string()),
            FileContent::Symlink(target) => Some(format!("symlink → {target}")),
            FileContent::Unreadable(err) => Some(format!("unreadable: {err}")),
        }
    }

    pub fn is_editable_text(&self) -> bool {
        matches!(self, FileContent::Code)
    }
}

/// `1.2 MB`-style size for cards and the inspector.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod file_content_tests {
    use super::*;

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
