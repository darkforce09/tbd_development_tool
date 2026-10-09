use crate::builder::{ProjectStats, ViewGranularity};
use memmap2::Mmap;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use studio_graph::Graph;

/// 16-byte magic identifier and format version header
pub const CACHE_MAGIC: &[u8; 16] = b"TBD_RKYV_V4\0\0\0\0\0";
/// Bump whenever extraction or graph building changes output, so cached graphs are rebuilt.
pub const EXTRACTOR_VERSION: u32 = 3;
const HEADER_SIZE: usize = 32;

/// Serializable wrapper combining the architecture graph and project metrics
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(derive(Debug))]
pub struct CachedProjectData {
    pub graph: Graph,
    pub stats: ProjectStats,
}

#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    Corrupted(String),
    Serialization(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "Cache I/O error: {}", e),
            CacheError::Corrupted(msg) => write!(f, "Cache corrupted: {}", msg),
            CacheError::Serialization(msg) => write!(f, "Cache serialization error: {}", msg),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// Resolves the user cache directory for a given project.
///
/// On Linux:   `~/.cache/tbd_studio/projects/<project_slug_hash>/`
/// On macOS:   `~/Library/Caches/tbd_studio/projects/<project_slug_hash>/`
/// On Windows: `%LOCALAPPDATA%\tbd_studio\cache\projects\<project_slug_hash>\`
pub fn user_cache_dir_for_project(project_root: &Path) -> PathBuf {
    let canonical = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());

    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let path_hash = hasher.finish();

    let folder_name = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("project");

    let safe_folder: String =
        folder_name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();

    let project_dir_name = format!("{}_{:016x}", safe_folder, path_hash);

    let base_cache = dirs::cache_dir().unwrap_or_else(|| std::env::temp_dir().join(".cache"));

    base_cache.join("tbd_studio").join("projects").join(project_dir_name)
}

/// Path to the graph cache file inside the user cache directory for a given granularity.
pub fn cache_file_path(project_root: &Path, granularity: ViewGranularity) -> PathBuf {
    user_cache_dir_for_project(project_root).join(format!("graph_cache_g{}.rkyv", granularity as u8))
}

/// Computes a fingerprint of the workspace to detect staleness: extractor version, git refs,
/// workspace manifests, and the size + mtime of every source file.
pub fn compute_workspace_fingerprint(project_root: &Path, granularity: ViewGranularity) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Hash project root and granularity
    if let Ok(c) = project_root.canonicalize() {
        c.hash(&mut hasher);
    } else {
        project_root.hash(&mut hasher);
    }
    (granularity as u8).hash(&mut hasher);
    EXTRACTOR_VERSION.hash(&mut hasher);

    // 1. Hash Git HEAD / ref if present (instant git tree fingerprint)
    let git_head = project_root.join(".git/HEAD");
    if let Ok(head_str) = std::fs::read_to_string(&git_head) {
        head_str.hash(&mut hasher);
        let ref_path = head_str.trim().trim_start_matches("ref: ").trim();
        let git_ref = project_root.join(".git").join(ref_path);
        if let Ok(ref_content) = std::fs::read_to_string(&git_ref) {
            ref_content.hash(&mut hasher);
        }
    }

    // 2. Hash key workspace manifests
    const MANIFESTS: &[&str] = &[
        "Cargo.lock",
        "Cargo.toml",
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "go.mod",
        "go.sum",
        "pyproject.toml",
        "requirements.txt",
        "Pipfile.lock",
        "pom.xml",
        "build.gradle",
        "CMakeLists.txt",
        "Makefile",
        "CLAUDE.md",
        "README.md",
    ];

    for manifest in MANIFESTS {
        let p = project_root.join(manifest);
        if let Ok(meta) = p.metadata() {
            meta.len().hash(&mut hasher);
            if let Ok(mtime) = meta.modified() {
                mtime.hash(&mut hasher);
            }
        }
    }

    // 3. Size + mtime of every discovered source file, so edits anywhere in the tree invalidate
    //    the cache (sorted for a stable order across git / walkdir discovery).
    let mut files = crate::project::discover_all_repository_files(project_root);
    files.sort();
    files.len().hash(&mut hasher);
    for path in &files {
        path.strip_prefix(project_root).unwrap_or(path).hash(&mut hasher);
        if let Ok(meta) = path.metadata() {
            meta.len().hash(&mut hasher);
            if let Ok(mtime) = meta.modified() {
                mtime.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

/// Saves the project Graph and Stats into the user cache directory using rkyv zero-copy serialization.
pub fn save_project_cache(
    project_root: &Path,
    granularity: ViewGranularity,
    graph: &Graph,
    stats: &ProjectStats,
) -> Result<PathBuf, CacheError> {
    let cache_dir = user_cache_dir_for_project(project_root);
    std::fs::create_dir_all(&cache_dir)?;
    let cache_file = cache_file_path(project_root, granularity);

    let fingerprint = compute_workspace_fingerprint(project_root, granularity);
    let cached_data = CachedProjectData { graph: graph.clone(), stats: stats.clone() };

    let aligned_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&cached_data).map_err(|e| CacheError::Serialization(e.to_string()))?;
    let rkyv_payload = aligned_bytes.as_slice();

    let payload_len = rkyv_payload.len() as u64;

    // Header layout:
    // [0..16]:   CACHE_MAGIC (16 bytes)
    // [16..24]:  fingerprint (8 bytes u64 LE)
    // [24..32]:  payload_len (8 bytes u64 LE)
    // [32..]:    rkyv byte buffer
    let mut buffer = Vec::with_capacity(HEADER_SIZE + rkyv_payload.len());
    buffer.extend_from_slice(CACHE_MAGIC);
    buffer.extend_from_slice(&fingerprint.to_le_bytes());
    buffer.extend_from_slice(&payload_len.to_le_bytes());
    buffer.extend_from_slice(rkyv_payload);

    crate::edit::atomic_write(&cache_file, &buffer)?;

    Ok(cache_file)
}

/// Attempts to load and reconstitute the Graph and Stats from the user cache directory.
/// Returns `Ok(Some((graph, stats)))` on cache hit and valid fingerprint.
/// Returns `Ok(None)` if the cache is missing, stale, or invalidated.
pub fn load_project_cache(
    project_root: &Path,
    granularity: ViewGranularity,
) -> Result<Option<(Graph, ProjectStats)>, CacheError> {
    let cache_file = cache_file_path(project_root, granularity);
    if !cache_file.is_file() {
        return Ok(None);
    }

    let file = File::open(&cache_file)?;
    let meta = file.metadata()?;
    if meta.len() < HEADER_SIZE as u64 {
        return Ok(None);
    }

    // Read and validate header
    let mut header_bytes = [0u8; HEADER_SIZE];
    {
        let mut reader = std::io::BufReader::new(&file);
        reader.read_exact(&mut header_bytes)?;
    }

    if &header_bytes[0..16] != CACHE_MAGIC {
        return Ok(None); // Magic mismatch (older format or corrupted)
    }

    let stored_fingerprint = u64::from_le_bytes(header_bytes[16..24].try_into().unwrap());
    let current_fingerprint = compute_workspace_fingerprint(project_root, granularity);
    if stored_fingerprint != current_fingerprint {
        return Ok(None); // Source files or manifests modified -> cache is stale
    }

    let stored_payload_len = u64::from_le_bytes(header_bytes[24..32].try_into().unwrap());
    if meta.len() < (HEADER_SIZE as u64 + stored_payload_len) {
        return Ok(None); // Truncated file
    }

    // Zero-copy access via memory mapping
    let mmap = unsafe { Mmap::map(&file)? };
    let payload = &mmap[HEADER_SIZE..HEADER_SIZE + stored_payload_len as usize];

    let archived = match rkyv::access::<ArchivedCachedProjectData, rkyv::rancor::Error>(payload) {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };

    let mut data: CachedProjectData = match rkyv::deserialize::<CachedProjectData, rkyv::rancor::Error>(archived) {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };

    data.graph.rebuild_fast_indices();
    Ok(Some((data.graph, data.stats)))
}

/// Deletes the user cache for the specified project.
pub fn clear_project_cache(project_root: &Path) -> Result<bool, CacheError> {
    let cache_dir = user_cache_dir_for_project(project_root);
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_graph::{DataType, NodeArchetype};

    #[test]
    fn test_user_cache_path_generation() {
        let path1 = Path::new("/run/media/system/Projects/test_proj_a");
        let path2 = Path::new("/run/media/system/Projects/test_proj_b");

        let cache1 = user_cache_dir_for_project(path1);
        let cache2 = user_cache_dir_for_project(path2);

        assert_ne!(cache1, cache2, "Different project paths must generate different cache dirs");
        assert!(cache1.to_string_lossy().contains("tbd_studio"));
        assert!(cache1.to_string_lossy().contains("projects"));
    }

    #[test]
    fn test_nested_file_edit_invalidates_cache() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        let deep_file = nested.join("deep.rs");
        std::fs::write(&deep_file, "pub fn deep() {}").unwrap();

        let granularity = ViewGranularity::FilesAndFolders;
        save_project_cache(dir.path(), granularity, &Graph::new(), &ProjectStats::default()).unwrap();
        assert!(load_project_cache(dir.path(), granularity).unwrap().is_some());

        std::fs::write(&deep_file, "pub fn deep() { let changed = 1; }").unwrap();
        assert!(load_project_cache(dir.path(), granularity).unwrap().is_none(), "nested edit must invalidate");
        clear_project_cache(dir.path()).unwrap();
    }

    #[test]
    fn test_save_and_load_cache_roundtrip() {
        let temp_proj_guard = tempfile::tempdir().unwrap();
        let temp_proj = temp_proj_guard.path().to_path_buf();
        let sample_file = temp_proj.join("lib.rs");
        std::fs::write(&sample_file, "pub fn foo() {}").unwrap();

        let mut graph = Graph::new();
        let nid = graph.add_node(
            "foo",
            NodeArchetype::Function,
            "A test function",
            None,
            vec![],
            vec![("out".to_string(), DataType::RustFlow)],
            [10.0, 20.0],
        );

        let granularity = ViewGranularity::AllItems;
        let stats = ProjectStats {
            project_name: "test_proj".to_string(),
            crate_count: 1,
            file_count: 1,
            node_count: 1,
            wire_count: 0,
            function_count: 1,
            type_count: 0,
        };

        // Save to cache
        let saved_path = save_project_cache(&temp_proj, granularity, &graph, &stats).expect("save cache");
        assert!(saved_path.is_file());

        // Load from cache
        let loaded = load_project_cache(&temp_proj, granularity).expect("load cache");
        assert!(loaded.is_some(), "Cache should be hit and valid");
        let (restored_graph, restored_stats) = loaded.unwrap();
        assert_eq!(restored_graph.nodes.len(), 1);
        assert!(restored_graph.nodes.contains_key(&nid));
        assert_eq!(restored_stats.project_name, "test_proj");
        assert_eq!(restored_stats.file_count, 1);

        // Test staleness detection: modify source file
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&sample_file, "pub fn foo() { let x = 42; }").unwrap();

        let loaded_after_edit = load_project_cache(&temp_proj, granularity).expect("load after edit");
        assert!(loaded_after_edit.is_none(), "Cache must be invalidated when source file changes");

        // Clean up
        let _ = clear_project_cache(&temp_proj);
    }
}
