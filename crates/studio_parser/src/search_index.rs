use std::collections::HashMap;
use studio_graph::{Graph, NodeArchetype, NodeId};

/// Fast in-memory symbol item for high-performance Spotlight search
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub node_id: NodeId,
    pub title: String,
    pub title_lower: String,
    pub crate_name: Option<String>,
    pub archetype: NodeArchetype,
}

/// In-memory Trigram and Substring Search Index optimized for DDR5 streaming
#[derive(Debug, Clone, Default)]
pub struct SymbolSearchIndex {
    items: Vec<SearchItem>,
    trigrams: HashMap<[u8; 3], Vec<u32>>,
}

impl SymbolSearchIndex {
    pub fn build(graph: &Graph) -> Self {
        let mut items = Vec::with_capacity(graph.nodes.len());
        let mut trigrams: HashMap<[u8; 3], Vec<u32>> = HashMap::new();

        for (&node_id, node) in &graph.nodes {
            let title_lower = node.title.to_lowercase();
            let idx = items.len() as u32;

            let bytes = title_lower.as_bytes();
            if bytes.len() >= 3 {
                for window in bytes.windows(3) {
                    let key = [window[0], window[1], window[2]];
                    let list = trigrams.entry(key).or_default();
                    if list.last() != Some(&idx) {
                        list.push(idx);
                    }
                }
            }

            items.push(SearchItem {
                node_id,
                title: node.title.clone(),
                title_lower,
                crate_name: node.crate_name.clone(),
                archetype: node.archetype,
            });
        }

        Self { items, trigrams }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&SearchItem> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.items.iter().take(limit).collect();
        }

        let q_bytes = q.as_bytes();
        if q_bytes.len() >= 3 {
            let first_trigram = [q_bytes[0], q_bytes[1], q_bytes[2]];
            if let Some(candidates) = self.trigrams.get(&first_trigram) {
                let mut results: Vec<&SearchItem> = candidates
                    .iter()
                    .filter_map(|&idx| self.items.get(idx as usize))
                    .filter(|item| item.title_lower.contains(&q))
                    .take(limit)
                    .collect();
                results.sort_by(|a, b| a.title.cmp(&b.title));
                return results;
            }
        }

        // Fast linear scan fallback for short 1-2 character queries
        let mut results: Vec<&SearchItem> = self
            .items
            .iter()
            .filter(|item| item.title_lower.contains(&q))
            .take(limit)
            .collect();
        results.sort_by(|a, b| a.title.cmp(&b.title));
        results
    }
}
