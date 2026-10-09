//! Graph algorithms for the layout: strongly connected components and an acyclic ordering.
//! Both are iterative (no recursion) and deterministic: they visit nodes in index order.

/// Strongly connected components of a directed graph given as adjacency lists (Tarjan's
/// algorithm with an explicit stack). Components come out in reverse topological order; each
/// component lists its members in ascending order.
pub fn strongly_connected(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let n = adj.len();
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next_index = 0;
    // (node, position of the next neighbour to visit)
    let mut work: Vec<(usize, usize)> = Vec::new();

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        work.push((root, 0));
        while let Some(&mut (v, ref mut next)) = work.last_mut() {
            if *next == 0 && index[v] == UNVISITED {
                index[v] = next_index;
                low[v] = next_index;
                next_index += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = adj[v].get(*next) {
                *next += 1;
                if index[w] == UNVISITED {
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                component.sort_unstable();
                components.push(component);
            }
        }
    }
    components
}

/// A node order in which as few edges as possible point backwards (greedy feedback arc set,
/// Eades–Lin–Smyth). For an acyclic graph this is a topological order. Ties break on the lowest
/// index, so the result is deterministic.
pub fn acyclic_order(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let mut out_deg = vec![0isize; n];
    let mut in_deg = vec![0isize; n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, succs) in adj.iter().enumerate() {
        for &v in succs {
            if u != v {
                out_deg[u] += 1;
                in_deg[v] += 1;
                preds[v].push(u);
            }
        }
    }
    let mut removed = vec![false; n];
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::new();
    let mut remaining = n;

    let remove = |v: usize, removed: &mut Vec<bool>, out_deg: &mut Vec<isize>, in_deg: &mut Vec<isize>| {
        removed[v] = true;
        for &w in &adj[v] {
            if w != v {
                in_deg[w] -= 1;
            }
        }
        for &u in &preds[v] {
            out_deg[u] -= 1;
        }
    };

    while remaining > 0 {
        let mut progressed = true;
        while progressed {
            progressed = false;
            // Sinks go to the end, sources to the front.
            for v in 0..n {
                if !removed[v] && out_deg[v] == 0 {
                    remove(v, &mut removed, &mut out_deg, &mut in_deg);
                    right.push(v);
                    remaining -= 1;
                    progressed = true;
                }
            }
            for v in 0..n {
                if !removed[v] && in_deg[v] == 0 {
                    remove(v, &mut removed, &mut out_deg, &mut in_deg);
                    left.push(v);
                    remaining -= 1;
                    progressed = true;
                }
            }
        }
        if remaining == 0 {
            break;
        }
        // Inside a cycle: take the node with the most outgoing minus incoming edges.
        let v =
            (0..n).filter(|&v| !removed[v]).max_by_key(|&v| (out_deg[v] - in_deg[v], std::cmp::Reverse(v))).unwrap();
        remove(v, &mut removed, &mut out_deg, &mut in_deg);
        left.push(v);
        remaining -= 1;
    }
    right.reverse();
    left.extend(right);
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cycles_and_singletons() {
        // 0 → 1 → 2 → 0, 2 → 3, 4 alone
        let adj = vec![vec![1], vec![2], vec![0, 3], vec![], vec![]];
        let mut comps = strongly_connected(&adj);
        comps.sort();
        assert_eq!(comps, vec![vec![0, 1, 2], vec![3], vec![4]]);
    }

    #[test]
    fn deep_chain_does_not_overflow_the_stack() {
        let n = 200_000;
        let adj: Vec<Vec<usize>> = (0..n).map(|i| if i + 1 < n { vec![i + 1] } else { vec![] }).collect();
        assert_eq!(strongly_connected(&adj).len(), n);
        let mut ring = adj.clone();
        ring[n - 1].push(0);
        assert_eq!(strongly_connected(&ring).len(), 1);
    }

    #[test]
    fn acyclic_order_is_topological_for_a_dag_and_breaks_cycles_once() {
        let dag = vec![vec![2], vec![2], vec![3], vec![]];
        let order = acyclic_order(&dag);
        let pos = |v: usize| order.iter().position(|&x| x == v).unwrap();
        assert!(pos(0) < pos(2) && pos(1) < pos(2) && pos(2) < pos(3));

        let ring = vec![vec![1], vec![2], vec![0]];
        let order = acyclic_order(&ring);
        let pos = |v: usize| order.iter().position(|&x| x == v).unwrap();
        let backwards = [(0, 1), (1, 2), (2, 0)].iter().filter(|&&(u, v)| pos(u) > pos(v)).count();
        assert_eq!(backwards, 1, "a 3-ring needs exactly one backward edge");
    }
}
