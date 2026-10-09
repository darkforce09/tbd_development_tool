use crate::model::{
    DataType, FileMemberNode, Graph, GroupCluster, NodeArchetype, Port, PortDirection, PortId,
};

/// Creates a showcase graph representing the CodeSee + CodeCanvas file/folder node system.
pub fn create_showcase_files_graph() -> Graph {
    let mut graph = Graph::new();

    // 1. Ingestion files
    let audio_rs = graph.add_node(
        "audio.rs",
        NodeArchetype::File,
        "AudioDemuxer implementation & PCM packet stream",
        Some("RS".to_string()),
        vec![],
        vec![("AudioPacket".to_string(), DataType::RustFlow)],
        [100.0, 100.0],
    );
    graph.set_node_metadata(
        audio_rs,
        Some("crates/media_demux/src/audio.rs".to_string()),
        Some(1),
        Some("media_demux".to_string()),
        Some("media_demux::audio".to_string()),
        Some("Extracts raw 48kHz PCM audio packets directly from incoming media transport streams.".to_string()),
        Some(r#"pub struct AudioDemuxer {
    stream_id: u32,
    sample_rate: u32,
}

impl AudioDemuxer {
    pub fn poll_next_packet(&mut self) -> Option<AudioPacket> {
        Some(AudioPacket::default())
    }
}"#.to_string()),
        Some("media_demux_src".to_string()),
    );
    let audio_poll_out = PortId(graph.next_raw_id());
    if let Some(n) = graph.nodes.get_mut(&audio_rs) {
        n.size = [220.0, 42.0];
        n.show_member_wires = true;
        n.outputs.push(Port {
            id: audio_poll_out,
            name: "poll_next_packet".to_string(),
            data_type: DataType::RustFlow,
            direction: PortDirection::Output,
        });
        n.member_nodes = vec![
            FileMemberNode::new(
                "struct:AudioDemuxer",
                "AudioDemuxer",
                NodeArchetype::Struct,
                "pub",
                "struct AudioDemuxer",
                25,
                "pub struct AudioDemuxer {\n    stream_id: u32,\n    sample_rate: u32,\n}",
                Some("Demuxes audio streams into PCM chunks.".to_string()),
            ),
            FileMemberNode::new(
                "fn:new",
                "new",
                NodeArchetype::Function,
                "pub",
                "fn new(stream_id: u32) -> Self",
                31,
                "pub fn new(stream_id: u32) -> Self {\n    Self { stream_id, sample_rate: 48_000 }\n}",
                Some("Constructs a new AudioDemuxer.".to_string()),
            ),
            FileMemberNode::new(
                "fn:poll_next_packet",
                "poll_next_packet",
                NodeArchetype::Function,
                "pub",
                "fn poll_next_packet(&mut self) -> Option<AudioPacket>",
                35,
                "pub fn poll_next_packet(&mut self) -> Option<AudioPacket> {\n    Some(AudioPacket::default())\n}",
                Some("Polls next raw 48kHz audio packet.".to_string()),
            ).with_ports(None, Some(audio_poll_out)),
        ];
    }

    let demux_lib_rs = graph.add_node(
        "lib.rs",
        NodeArchetype::File,
        "Media demux module root & exports",
        Some("RS".to_string()),
        vec![],
        vec![("exports".to_string(), DataType::RustFlow)],
        [100.0, 156.0],
    );
    graph.set_node_metadata(
        demux_lib_rs,
        Some("crates/media_demux/src/lib.rs".to_string()),
        Some(1),
        Some("media_demux".to_string()),
        Some("media_demux".to_string()),
        Some("Crate root re-exporting AudioDemuxer.".to_string()),
        Some("pub mod audio;\npub use audio::AudioDemuxer;\n".to_string()),
        Some("media_demux_src".to_string()),
    );
    if let Some(n) = graph.nodes.get_mut(&demux_lib_rs) {
        n.size = [220.0, 42.0];
        n.member_nodes = vec![
            FileMemberNode::new(
                "mod:audio",
                "audio",
                NodeArchetype::Module,
                "pub",
                "pub mod audio;",
                1,
                "pub mod audio;",
                None,
            ),
        ];
    }

    // 2. Speech Engine files
    let whisper_rs = graph.add_node(
        "whisper.rs",
        NodeArchetype::File,
        "Whisper OCR speech model inference",
        Some("RS".to_string()),
        vec![("packet".to_string(), DataType::RustFlow)],
        vec![("tokens".to_string(), DataType::RustFlow)],
        [420.0, 100.0],
    );
    graph.set_node_metadata(
        whisper_rs,
        Some("crates/speech_engine/src/whisper.rs".to_string()),
        Some(1),
        Some("speech_engine".to_string()),
        Some("speech_engine::whisper".to_string()),
        Some("Processes AudioPackets through Whisper model to emit subtitle text tokens.".to_string()),
        Some(r#"pub struct WhisperOcr {
    confidence: f32,
}

impl WhisperOcr {
    pub fn transcribe(&self, packet: AudioPacket) -> SubtitleText {
        SubtitleText::new()
    }
}"#.to_string()),
        Some("speech_engine_src".to_string()),
    );
    let whisper_transcribe_in = PortId(graph.next_raw_id());
    let whisper_transcribe_out = PortId(graph.next_raw_id());
    if let Some(n) = graph.nodes.get_mut(&whisper_rs) {
        n.size = [280.0, 100.0];
        n.is_dropdown_expanded = true;
        n.show_member_wires = true;
        n.inputs.push(Port {
            id: whisper_transcribe_in,
            name: "transcribe:in".to_string(),
            data_type: DataType::RustFlow,
            direction: PortDirection::Input,
        });
        n.outputs.push(Port {
            id: whisper_transcribe_out,
            name: "transcribe:out".to_string(),
            data_type: DataType::RustFlow,
            direction: PortDirection::Output,
        });
        n.member_nodes = vec![
            FileMemberNode::new(
                "struct:WhisperOcr",
                "WhisperOcr",
                NodeArchetype::Struct,
                "pub",
                "struct WhisperOcr",
                12,
                "pub struct WhisperOcr {\n    confidence: f32,\n}",
                Some("Whisper speech model inference context.".to_string()),
            ),
            FileMemberNode::new(
                "fn:transcribe",
                "transcribe",
                NodeArchetype::Function,
                "pub",
                "fn transcribe(&self, packet: AudioPacket) -> SubtitleText",
                18,
                "pub fn transcribe(&self, packet: AudioPacket) -> SubtitleText {\n    SubtitleText::new()\n}",
                Some("Transcribes audio packet into subtitle tokens.".to_string()),
            ).with_ports(Some(whisper_transcribe_in), Some(whisper_transcribe_out)),
        ];
    }

    let speech_lib_rs = graph.add_node(
        "lib.rs",
        NodeArchetype::File,
        "Speech engine module root",
        Some("RS".to_string()),
        vec![],
        vec![("exports".to_string(), DataType::RustFlow)],
        [420.0, 156.0],
    );
    graph.set_node_metadata(
        speech_lib_rs,
        Some("crates/speech_engine/src/lib.rs".to_string()),
        Some(1),
        Some("speech_engine".to_string()),
        Some("speech_engine".to_string()),
        Some("Crate root re-exporting WhisperOcr.".to_string()),
        Some("pub mod whisper;\npub use whisper::WhisperOcr;\n".to_string()),
        Some("speech_engine_src".to_string()),
    );
    if let Some(n) = graph.nodes.get_mut(&speech_lib_rs) {
        n.size = [220.0, 42.0];
        n.member_nodes = vec![
            FileMemberNode::new(
                "mod:whisper",
                "whisper",
                NodeArchetype::Module,
                "pub",
                "pub mod whisper;",
                1,
                "pub mod whisper;",
                None,
            ),
        ];
    }

    // 3. Video Core files
    let cache_rs = graph.add_node(
        "cache.rs",
        NodeArchetype::File,
        "VRAM video frame cache ring buffer",
        Some("RS".to_string()),
        vec![],
        vec![("frames".to_string(), DataType::RustFlow)],
        [100.0, 360.0],
    );
    graph.set_node_metadata(
        cache_rs,
        Some("crates/video_core/src/cache.rs".to_string()),
        Some(1),
        Some("video_core".to_string()),
        Some("video_core::cache".to_string()),
        Some("Hardware-accelerated ring buffer maintaining recent 4K video frames.".to_string()),
        Some(r#"pub struct VideoFrameCache {
    buffer: Vec<ArchivedFrame>,
}

impl VideoFrameCache {
    pub fn push_frame(&mut self, frame: ArchivedFrame) {
        self.buffer.push(frame);
    }
}"#.to_string()),
        Some("video_core_src".to_string()),
    );
    if let Some(n) = graph.nodes.get_mut(&cache_rs) {
        n.size = [220.0, 42.0];
        n.member_nodes = vec![
            FileMemberNode::new(
                "struct:VideoFrameCache",
                "VideoFrameCache",
                NodeArchetype::Struct,
                "pub",
                "struct VideoFrameCache",
                8,
                "pub struct VideoFrameCache {\n    buffer: Vec<ArchivedFrame>,\n}",
                Some("Circular ring buffer for decompressed video frames in VRAM.".to_string()),
            ),
            FileMemberNode::new(
                "fn:push_frame",
                "push_frame",
                NodeArchetype::Function,
                "pub",
                "fn push_frame(&mut self, frame: ArchivedFrame)",
                14,
                "pub fn push_frame(&mut self, frame: ArchivedFrame) {\n    self.buffer.push(frame);\n}",
                Some("Appends decompressed frame to buffer.".to_string()),
            ),
        ];
    }

    // 4. Display Compositor files
    let surface_rs = graph.add_node(
        "surface.rs",
        NodeArchetype::File,
        "Hardware swapchain & direct scanout",
        Some("RS".to_string()),
        vec![("tokens".to_string(), DataType::RustFlow), ("frames".to_string(), DataType::RustFlow)],
        vec![("framebuffer".to_string(), DataType::RustFlow)],
        [420.0, 360.0],
    );
    graph.set_node_metadata(
        surface_rs,
        Some("crates/display/src/surface.rs".to_string()),
        Some(1),
        Some("display".to_string()),
        Some("display::surface".to_string()),
        Some("Vulkan surface presenter presenting video frames to monitor.".to_string()),
        Some(r#"pub struct DisplaySurface {
    vsync: bool,
}

impl DisplaySurface {
    pub fn present_frame(&mut self, frame: CompositeFrame) {
        // Scanout to physical display surface
    }
}"#.to_string()),
        Some("display_src".to_string()),
    );
    let surface_present_in = PortId(graph.next_raw_id());
    if let Some(n) = graph.nodes.get_mut(&surface_rs) {
        n.size = [220.0, 42.0];
        n.show_member_wires = true;
        n.inputs.push(Port {
            id: surface_present_in,
            name: "present_frame:in".to_string(),
            data_type: DataType::RustFlow,
            direction: PortDirection::Input,
        });
        n.member_nodes = vec![
            FileMemberNode::new(
                "struct:DisplaySurface",
                "DisplaySurface",
                NodeArchetype::Struct,
                "pub",
                "struct DisplaySurface",
                10,
                "pub struct DisplaySurface {\n    vsync: bool,\n}",
                Some("Hardware surface presentation presenter.".to_string()),
            ),
            FileMemberNode::new(
                "fn:present_frame",
                "present_frame",
                NodeArchetype::Function,
                "pub",
                "fn present_frame(&mut self, frame: CompositeFrame)",
                16,
                "pub fn present_frame(&mut self, frame: CompositeFrame) {\n    // Scanout to physical display surface\n}",
                Some("Presents frame to monitor surface.".to_string()),
            ).with_ports(Some(surface_present_in), None),
        ];
    }

    // Connect file dependency edges
    let audio_out = graph.nodes[&audio_rs].outputs[0].id;
    let whisper_in = graph.nodes[&whisper_rs].inputs[0].id;
    graph.connect_labeled(
        audio_rs,
        audio_out,
        whisper_rs,
        whisper_in,
        Some("use speech_engine::WhisperOcr".to_string()),
        Some(1),
        Some(18),
    );

    let cache_out = graph.nodes[&cache_rs].outputs[0].id;
    let surface_in_frame = graph.nodes[&surface_rs].inputs[1].id;
    graph.connect_labeled(
        cache_rs,
        cache_out,
        surface_rs,
        surface_in_frame,
        Some("use video_core::ArchivedFrame".to_string()),
        Some(2),
        Some(24),
    );

    let whisper_out = graph.nodes[&whisper_rs].outputs[0].id;
    let surface_in_tokens = graph.nodes[&surface_rs].inputs[0].id;
    graph.connect_labeled(
        whisper_rs,
        whisper_out,
        surface_rs,
        surface_in_tokens,
        Some("call surface.present_frame()".to_string()),
        Some(3),
        Some(36),
    );

    // Connect fine-grained sub-node dependency edges
    graph.connect_labeled(
        audio_rs,
        audio_poll_out,
        whisper_rs,
        whisper_transcribe_in,
        Some("poll_packet() -> transcribe()".to_string()),
        Some(1),
        Some(12),
    );

    graph.connect_labeled(
        whisper_rs,
        whisper_transcribe_out,
        surface_rs,
        surface_present_in,
        Some("transcribe() -> present_frame()".to_string()),
        Some(3),
        Some(20),
    );

    // Build hierarchical clusters
    // Media Demux
    let mut c_demux_root = GroupCluster::new("media_demux", "crates/media_demux", "crate", 0);
    c_demux_root.subtitle = Some("Raw stream extraction crate".to_string());
    c_demux_root.child_cluster_ids = vec!["media_demux_src".to_string()];

    let mut c_demux_src = GroupCluster::new("media_demux_src", "src", "directory", 1);
    c_demux_src.parent_id = Some("media_demux".to_string());
    c_demux_src.node_ids = vec![audio_rs, demux_lib_rs];

    // Speech Engine
    let mut c_speech_root = GroupCluster::new("speech_engine", "crates/speech_engine", "crate", 0);
    c_speech_root.subtitle = Some("Speech transcription crate".to_string());
    c_speech_root.child_cluster_ids = vec!["speech_engine_src".to_string()];

    let mut c_speech_src = GroupCluster::new("speech_engine_src", "src", "directory", 1);
    c_speech_src.parent_id = Some("speech_engine".to_string());
    c_speech_src.node_ids = vec![whisper_rs, speech_lib_rs];

    // Video Core
    let mut c_video_root = GroupCluster::new("video_core", "crates/video_core", "crate", 0);
    c_video_root.subtitle = Some("Video frame memory core".to_string());
    c_video_root.child_cluster_ids = vec!["video_core_src".to_string()];

    let mut c_video_src = GroupCluster::new("video_core_src", "src", "directory", 1);
    c_video_src.parent_id = Some("video_core".to_string());
    c_video_src.node_ids = vec![cache_rs];

    // Display
    let mut c_display_root = GroupCluster::new("display", "crates/display", "crate", 0);
    c_display_root.subtitle = Some("Hardware presentation swapchain".to_string());
    c_display_root.child_cluster_ids = vec!["display_src".to_string()];

    let mut c_display_src = GroupCluster::new("display_src", "src", "directory", 1);
    c_display_src.parent_id = Some("display".to_string());
    c_display_src.node_ids = vec![surface_rs];

    graph.clusters = vec![
        c_demux_root, c_demux_src,
        c_speech_root, c_speech_src,
        c_video_root, c_video_src,
        c_display_root, c_display_src,
    ];

    graph.update_cluster_bounds();
    graph
}
