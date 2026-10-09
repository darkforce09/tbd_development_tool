use crate::model::{DataType, EdgeKind, Graph, GroupCluster, NodeArchetype};

/// Creates the pre-populated showcase pipeline scenario.
pub fn create_showcase_graph() -> Graph {
    let mut graph = Graph::new();

    // 1. Audio Demuxer (Ingress / Cyan)
    let audio_demuxer = graph.add_node(
        "Audio Demuxer",
        NodeArchetype::Ingress,
        "Extracts PCM raw audio chunks from stream",
        Some("INGRESS-01".to_string()),
        vec![],
        vec![("AudioPacket".to_string(), DataType::Audio)],
        [100.0, 140.0],
    );
    graph.set_node_metadata(
        audio_demuxer,
        Some("crates/media_demux/src/audio.rs".to_string()),
        Some(18),
        Some("media_demux".to_string()),
        Some("media_demux::audio".to_string()),
        Some("Extracts raw 48kHz PCM audio packets directly from incoming media transport streams.".to_string()),
        Some(
            r#"pub struct AudioDemuxer {
    stream_id: u32,
    sample_rate: u32,
}

impl AudioDemuxer {
    pub fn new(stream_id: u32) -> Self {
        Self { stream_id, sample_rate: 48_000 }
    }

    pub fn poll_next_packet(&mut self) -> Option<AudioPacket> {
        // Demux next chunk from buffer
        Some(AudioPacket::default())
    }
}"#
            .to_string(),
        ),
        Some("ingestion".to_string()),
    );

    // 2. Whisper Speech OCR (Compute / Purple)
    let whisper_ocr = graph.add_node(
        "Whisper Speech OCR",
        NodeArchetype::Compute,
        "Generates timed subtitle text tokens",
        Some("TENSOR-FP16".to_string()),
        vec![("AudioPacket".to_string(), DataType::Audio)],
        vec![("SubtitleText".to_string(), DataType::Text)],
        [500.0, 100.0],
    );
    graph.set_node_metadata(
        whisper_ocr,
        Some("crates/speech_engine/src/whisper.rs".to_string()),
        Some(42),
        Some("speech_engine".to_string()),
        Some("speech_engine::whisper".to_string()),
        Some("Processes AudioPackets through Whisper model to emit real-time subtitle text tokens.".to_string()),
        Some(
            r#"pub struct WhisperOcr {
    model_weights: PathBuf,
    confidence_threshold: f32,
}

impl WhisperOcr {
    pub async fn transcribe(&self, packet: AudioPacket) -> Result<SubtitleText, OcrError> {
        let tokens = run_inference_fp16(&packet.pcm_data).await?;
        Ok(SubtitleText::from_tokens(tokens))
    }
}"#
            .to_string(),
        ),
        Some("inference".to_string()),
    );

    // 3. Video Frame Cache (State / Amber)
    let frame_cache = graph.add_node(
        "Video Frame Cache",
        NodeArchetype::State,
        "Circular buffer of 4K decompressed frames",
        Some("VRAM-RING".to_string()),
        vec![],
        vec![("ArchivedFrame".to_string(), DataType::Vision)],
        [100.0, 420.0],
    );
    graph.set_node_metadata(
        frame_cache,
        Some("crates/video_core/src/cache.rs".to_string()),
        Some(27),
        Some("video_core".to_string()),
        Some("video_core::cache".to_string()),
        Some("Hardware-accelerated ring buffer maintaining recent 4K decompressed video frames in VRAM.".to_string()),
        Some(
            r#"pub struct VideoFrameCache {
    ring_size: usize,
    buffer: Vec<ArchivedFrame>,
}

impl VideoFrameCache {
    pub fn push_frame(&mut self, frame: ArchivedFrame) {
        if self.buffer.len() >= self.ring_size {
            self.buffer.remove(0);
        }
        self.buffer.push(frame);
    }
}"#
            .to_string(),
        ),
        Some("ingestion".to_string()),
    );

    // 4. Inpainting Engine (Compute / Purple)
    let inpainting = graph.add_node(
        "Inpainting Engine",
        NodeArchetype::Compute,
        "Merges subtitle overlays onto active frames",
        Some("CUDA-OPT".to_string()),
        vec![("SubtitleText".to_string(), DataType::Text), ("ArchivedFrame".to_string(), DataType::Vision)],
        vec![("CompositeFrame".to_string(), DataType::Composite)],
        [500.0, 380.0],
    );
    graph.set_node_metadata(
        inpainting,
        Some("crates/compositor/src/inpaint.rs".to_string()),
        Some(65),
        Some("compositor".to_string()),
        Some("compositor::inpaint".to_string()),
        Some("Performs CUDA shader compositing to render subtitle overlays on top of video frames.".to_string()),
        Some(
            r#"pub struct InpaintingEngine {
    font_atlas: TextureAtlas,
}

impl InpaintingEngine {
    pub fn composite(&self, text: &SubtitleText, frame: &ArchivedFrame) -> CompositeFrame {
        let mut out = frame.clone();
        render_subtitles(&mut out, text);
        out
    }
}"#
            .to_string(),
        ),
        Some("inference".to_string()),
    );

    // 5. Display Surface (Egress / Emerald)
    let display_surface = graph.add_node(
        "Display Surface",
        NodeArchetype::Egress,
        "Hardware surface compositor presentation",
        Some("WAYLAND-DRM".to_string()),
        vec![("CompositeFrame".to_string(), DataType::Composite)],
        vec![],
        [900.0, 260.0],
    );
    graph.set_node_metadata(
        display_surface,
        Some("crates/compositor/src/surface.rs".to_string()),
        Some(12),
        Some("compositor".to_string()),
        Some("compositor::surface".to_string()),
        Some("Direct Wayland DRM surface presentation layer with vsync synchronization.".to_string()),
        Some(
            r#"pub struct DisplaySurface {
    drm_fd: RawFd,
}

impl DisplaySurface {
    pub fn present(&mut self, frame: &CompositeFrame) -> Result<(), SurfaceError> {
        drm_page_flip(self.drm_fd, frame.buffer_id)?;
        Ok(())
    }
}"#
            .to_string(),
        ),
        Some("output".to_string()),
    );

    // Wire up edges with labels and step numbers:
    // Step 1: Audio Demuxer -> Whisper OCR
    let audio_out_port = graph.nodes[&audio_demuxer].outputs[0].id;
    let whisper_in_port = graph.nodes[&whisper_ocr].inputs[0].id;
    graph.connect_labeled(
        audio_demuxer,
        audio_out_port,
        whisper_ocr,
        whisper_in_port,
        EdgeKind::Call,
        Some("PCM Stream".to_string()),
        Some(1),
        Some(26),
    );

    // Step 2: Whisper OCR -> Inpainting Engine
    let whisper_out_port = graph.nodes[&whisper_ocr].outputs[0].id;
    let inpainting_text_port = graph.nodes[&inpainting].inputs[0].id;
    graph.connect_labeled(
        whisper_ocr,
        whisper_out_port,
        inpainting,
        inpainting_text_port,
        EdgeKind::Call,
        Some("Subtitle Tokens".to_string()),
        Some(2),
        Some(50),
    );

    // Step 3: Video Frame Cache -> Inpainting Engine
    let frame_out_port = graph.nodes[&frame_cache].outputs[0].id;
    let inpainting_frame_port = graph.nodes[&inpainting].inputs[1].id;
    graph.connect_labeled(
        frame_cache,
        frame_out_port,
        inpainting,
        inpainting_frame_port,
        EdgeKind::Call,
        Some("4K Frames".to_string()),
        Some(3),
        Some(32),
    );

    // Step 4: Inpainting Engine -> Display Surface
    let composite_out_port = graph.nodes[&inpainting].outputs[0].id;
    let display_in_port = graph.nodes[&display_surface].inputs[0].id;
    graph.connect_labeled(
        inpainting,
        composite_out_port,
        display_surface,
        display_in_port,
        EdgeKind::Call,
        Some("Composite Buffer".to_string()),
        Some(4),
        Some(72),
    );

    // Add CodeSee-style group clusters
    let mut c_ingest = GroupCluster::new("ingestion", "Ingress & Buffers", "Ingress Subsystem", 0);
    c_ingest.subtitle = Some("Extracts raw audio and video frames".to_string());
    c_ingest.node_ids = vec![audio_demuxer, frame_cache];

    let mut c_infer = GroupCluster::new("inference", "ML & Compute Pipeline", "Compute Subsystem", 1);
    c_infer.subtitle = Some("Speech inference & subtitle generation".to_string());
    c_infer.node_ids = vec![whisper_ocr, inpainting];

    let mut c_output = GroupCluster::new("output", "Compositor Output", "Egress Subsystem", 2);
    c_output.subtitle = Some("Hardware frame scanout".to_string());
    c_output.node_ids = vec![display_surface];

    graph.clusters = vec![c_ingest, c_infer, c_output];

    graph.update_cluster_bounds();

    graph
}
