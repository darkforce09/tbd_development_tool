//! The final order of steps and links, the flows through them, their groups and the stats.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use studio_graph::EvidenceTier;

use super::build::Found;
use super::model::{Flow, FlowGroup, FlowGroupKind, Link, LinkKind, Pipeline, PipelineStats, Step, StepKind};

/// The most BFS layers a flow holds (layer 0 included).
pub const MAX_LAYERS: usize = 12;
/// The most steps a flow holds.
pub const MAX_STEPS: usize = 400;

/// Sorts steps and links with total tie-breaks, remaps indices, and builds the flows, groups and stats.
pub(crate) fn assemble(found: Found) -> Pipeline {
    let Found { steps, links, entries, .. } = &found;

    // Steps: by kind, path, line, title, detail, language, end line.
    let mut order: Vec<usize> = (0..steps.len()).collect();
    let step_key =
        |s: &Step| (s.kind, s.path.clone(), s.line, s.title.clone(), s.detail.clone(), s.language.clone(), s.line_end);
    order.sort_by(|&a, &b| step_key(&steps[a]).cmp(&step_key(&steps[b])));
    let mut new_index = vec![0usize; steps.len()];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    let steps: Vec<Step> = order.iter().map(|&i| steps[i].clone()).collect();

    // Links: by (from, to, kind) after remapping, which is unique.
    let mut links: Vec<Link> = links
        .iter()
        .map(|l| {
            let mut l = l.clone();
            l.from = new_index[l.from];
            l.to = new_index[l.to];
            l
        })
        .collect();
    links.sort_by_key(|l| (l.from, l.to, l.kind));
    let mut entries: Vec<usize> = entries.iter().map(|&i| new_index[i]).collect();
    entries.sort();
    entries.dedup();

    // Outgoing links usable by flows (Proven or Possible set), and incoming Requests per route.
    let usable = |l: &Link| matches!(l.provenance.tier, EvidenceTier::Proven | EvidenceTier::PossibleSet);
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    let mut requests_in: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    for (li, l) in links.iter().enumerate() {
        if !usable(l) {
            continue;
        }
        if l.kind == LinkKind::Requests {
            requests_in[l.to].push(li);
        } else {
            outgoing[l.from].push(li);
        }
    }

    let mut flows: Vec<Flow> =
        entries.iter().map(|&entry| flow(entry, &steps, &links, &outgoing, &requests_in)).collect();

    // Groups: a flow is in the first that fits — crosses languages, has no links, then by its entry's kind.
    let mut groups: BTreeMap<(FlowGroupKind, String), Vec<usize>> = BTreeMap::new();
    for (fi, f) in flows.iter().enumerate() {
        let entry = &steps[f.entry];
        let key = if f.crosses_languages {
            (FlowGroupKind::CrossLanguage, "Cross-language".to_string())
        } else if f.links.is_empty() {
            (FlowGroupKind::NoLinks, "Entry points with no resolved links".to_string())
        } else {
            match entry.kind {
                StepKind::Route => (FlowGroupKind::Endpoints, entry.path.to_string_lossy().replace('\\', "/")),
                StepKind::Main => (FlowGroupKind::Mains, "Programs".to_string()),
                _ => (FlowGroupKind::Commands, "Commands".to_string()),
            }
        };
        groups.entry(key).or_default().push(fi);
    }
    // Flows inside a group: by router or crate path, then template or path, then name.
    let flow_key = |f: &Flow| {
        let e = &steps[f.entry];
        (e.path.clone(), e.title.clone(), f.name.clone(), e.line, f.entry)
    };
    let mut flow_order: Vec<usize> = Vec::with_capacity(flows.len());
    let mut group_list: Vec<FlowGroup> = Vec::new();
    for ((kind, name), mut members) in groups {
        members.sort_by(|&a, &b| flow_key(&flows[a]).cmp(&flow_key(&flows[b])));
        let start = flow_order.len();
        flow_order.extend(members);
        group_list.push(FlowGroup {
            kind,
            name,
            flows: (start..flow_order.len()).collect(),
            expanded_by_default: kind == FlowGroupKind::CrossLanguage,
        });
    }
    for (gi, g) in group_list.iter().enumerate() {
        for &fi in &g.flows {
            flows[flow_order[fi]].group = gi;
        }
    }
    let flows: Vec<Flow> = flow_order.iter().map(|&i| flows[i].clone()).collect();

    let mut links_by_tier = [0usize; 4];
    for l in &links {
        links_by_tier[match l.provenance.tier {
            EvidenceTier::Proven => 0,
            EvidenceTier::PossibleSet => 1,
            EvidenceTier::Observed => 2,
            EvidenceTier::Unresolved => 3,
        }] += 1;
    }
    let stats = PipelineStats {
        routes: found.routes,
        route_tags: found.route_tags,
        contract_tags: found.contract_tags,
        entry_points: entries.len(),
        flows: flows.len(),
        steps: steps.len(),
        links_by_tier,
        crossings: links.iter().filter(|l| l.crossing.is_some()).count(),
        handler_tags_agree: found.handler_tags_agree,
        handler_tags_disagree: found.handler_tags_disagree,
        elapsed_ms: 0,
    };
    Pipeline { steps, links, flows, groups: group_list, stats }
}

/// One flow: layer 0 holds the senders that request the entry and the entry; then BFS forward over usable links,
/// within [`MAX_LAYERS`] and [`MAX_STEPS`]. `truncated` counts the reachable steps left out.
fn flow(entry: usize, steps: &[Step], links: &[Link], outgoing: &[Vec<usize>], requests_in: &[Vec<usize>]) -> Flow {
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut layer0: Vec<usize> = Vec::new();
    for &li in &requests_in[entry] {
        let from = links[li].from;
        used.insert(li);
        if seen.insert(from) {
            layer0.push(from);
        }
    }
    layer0.sort();
    seen.insert(entry);
    layer0.push(entry);
    let mut kept: BTreeSet<usize> = layer0.iter().copied().collect();
    let mut layers = vec![layer0];
    let mut count = kept.len();

    // BFS over everything reachable; steps past the caps are counted, not kept.
    let mut queue: VecDeque<(usize, usize)> = VecDeque::from([(entry, 0)]);
    let mut truncated = 0usize;
    while let Some((step, depth)) = queue.pop_front() {
        let mut out: Vec<usize> = outgoing[step].clone();
        out.sort_by_key(|&li| (links[li].to, li));
        for li in out {
            let to = links[li].to;
            if kept.contains(&step) && kept.contains(&to) {
                used.insert(li);
            }
            if !seen.insert(to) {
                continue;
            }
            let layer = depth + 1;
            if kept.contains(&step) && layer < MAX_LAYERS && count < MAX_STEPS {
                if layers.len() <= layer {
                    layers.push(Vec::new());
                }
                layers[layer].push(to);
                kept.insert(to);
                count += 1;
                used.insert(li);
            } else {
                truncated += 1;
            }
            queue.push_back((to, layer));
        }
    }
    // Layer 0 stays [senders…, entry]: the entry is last, always; the later layers are sorted.
    for layer in layers.iter_mut().skip(1) {
        layer.sort();
    }
    let links_used: Vec<usize> = used.into_iter().collect();
    let crosses_languages = links_used.iter().any(|&li| links[li].crossing.is_some());
    Flow { name: steps[entry].title.clone(), entry, group: 0, layers, links: links_used, truncated, crosses_languages }
}
