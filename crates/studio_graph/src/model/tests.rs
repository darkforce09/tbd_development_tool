use super::*;

#[test]
fn test_rkyv_graph_roundtrip() {
    let mut graph = Graph::new();
    let n1 = graph.add_node(
        "NodeA",
        NodeArchetype::Function,
        "Description A",
        Some("FN".to_string()),
        vec![("in1".to_string(), DataType::RustFlow)],
        vec![("out1".to_string(), DataType::RustFlow)],
        [10.0, 20.0],
    );
    let n2 = graph.add_node(
        "NodeB",
        NodeArchetype::Struct,
        "Description B",
        None,
        vec![("in2".to_string(), DataType::RustFlow)],
        vec![],
        [150.0, 80.0],
    );

    let p_out = graph.nodes[&n1].outputs[0].id;
    let p_in = graph.nodes[&n2].inputs[0].id;
    let edge_id = graph.connect(n1, p_out, n2, p_in).expect("connect");

    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.isolated_nodes_count(), 0);

    // Serialize with rkyv
    let bytes = graph.to_rkyv_bytes().expect("serialize to rkyv");
    assert!(!bytes.is_empty());

    // Zero-copy access check
    let archived = rkyv::access::<ArchivedGraph, rkyv::rancor::Error>(&bytes).expect("zero-copy access");
    assert_eq!(archived.nodes.len(), 2);
    assert_eq!(archived.edges.len(), 1);

    // Full reconstitution
    let restored = Graph::from_rkyv_bytes(&bytes).expect("deserialize from rkyv");
    assert_eq!(restored.nodes.len(), 2);
    assert_eq!(restored.edges.len(), 1);
    assert_eq!(restored.edges[0].id, edge_id);
    assert_eq!(restored.isolated_nodes_count(), 0);
    assert!(restored.is_port_connected(n1, p_out));
    assert!(restored.is_port_connected(n2, p_in));
}

#[test]
fn test_hierarchical_cluster_bounds_and_collapse() {
    let mut graph = Graph::new();
    let n1 = graph.add_node("FileA", NodeArchetype::File, "", None, vec![], vec![], [100.0, 100.0]);
    let n2 = graph.add_node("FileB", NodeArchetype::File, "", None, vec![], vec![], [200.0, 150.0]);

    let mut child_cluster = GroupCluster::new("child", "widgets", "Directory", 1);
    child_cluster.node_ids = vec![n1, n2];
    child_cluster.parent_id = Some("parent".to_string());

    let mut parent_cluster = GroupCluster::new("parent", "ui", "Crate", 0);
    parent_cluster.child_cluster_ids = vec!["child".to_string()];

    graph.clusters = vec![parent_cluster, child_cluster];
    graph.update_cluster_bounds();

    let child = graph.clusters.iter().find(|c| c.id == "child").unwrap();
    assert!(child.size[0] > 0.0);
    assert!(child.size[1] > 0.0);

    let parent = graph.clusters.iter().find(|c| c.id == "parent").unwrap();
    // Parent must strictly enclose child!
    assert!(parent.position[0] <= child.position[0]);
    assert!(parent.position[1] <= child.position[1]);
    assert!(parent.position[0] + parent.size[0] >= child.position[0] + child.size[0]);
    assert!(parent.position[1] + parent.size[1] >= child.position[1] + child.size[1]);

    // Toggle collapse on child
    assert!(graph.toggle_cluster_collapse("child"));
    let child_collapsed = graph.clusters.iter().find(|c| c.id == "child").unwrap();
    assert!(child_collapsed.is_collapsed);
    assert_eq!(child_collapsed.size, [220.0, 38.0]);
    assert!(graph.is_node_in_collapsed_cluster(n1));
}

#[test]
fn test_file_node_member_nodes_and_dropdown() {
    let mut graph = Graph::new();
    let nid = graph.add_node("test.rs", NodeArchetype::File, "Test file", Some("RS".to_string()), vec![], vec![], [10.0, 10.0]);

    let p_in = PortId(100);
    let p_out = PortId(101);
    if let Some(n) = graph.nodes.get_mut(&nid) {
        n.member_nodes.push(
            FileMemberNode::new(
                "fn:foo",
                "foo",
                NodeArchetype::Function,
                "pub",
                "fn foo() -> u32",
                12,
                "pub fn foo() -> u32 { 42 }",
                Some("Returns 42".to_string()),
            )
            .with_ports(Some(p_in), Some(p_out)),
        );
        n.is_dropdown_expanded = true;
        n.show_member_wires = true;
        n.expanded_member_id = Some("fn:foo".to_string());
    }

    // Verify rkyv serialize/deserialize roundtrip preserves members and ports
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&graph).expect("serialize");
    let archived = rkyv::access::<ArchivedGraph, rkyv::rancor::Error>(&bytes).expect("access");
    let deserialized: Graph = rkyv::deserialize::<Graph, rkyv::rancor::Error>(archived).expect("deserialize");

    let n = &deserialized.nodes[&nid];
    assert_eq!(n.archetype, NodeArchetype::File);
    assert!(n.is_dropdown_expanded);
    assert!(n.show_member_wires);
    assert_eq!(n.expanded_member_id.as_deref(), Some("fn:foo"));
    assert_eq!(n.member_nodes.len(), 1);
    assert_eq!(n.member_nodes[0].name, "foo");
    assert_eq!(n.member_nodes[0].line_number, 12);
    assert_eq!(n.member_nodes[0].in_port_id, Some(p_in));
    assert_eq!(n.member_nodes[0].out_port_id, Some(p_out));
}

#[test]
fn test_node_expanded_tab_and_scroll_roundtrip() {
    let mut graph = Graph::new();
    let nid = graph.add_node("readme.md", NodeArchetype::File, "Readme", Some("MD".to_string()), vec![], vec![], [0.0, 0.0]);

    let node = &graph.nodes[&nid];
    assert_eq!(node.expanded_tab, 0);
    assert_eq!(node.scroll_offset_y, 0.0);

    if let Some(n) = graph.nodes.get_mut(&nid) {
        n.expanded_tab = 1;
        n.scroll_offset_y = 45.5;
    }

    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&graph).expect("serialize");
    let archived = rkyv::access::<ArchivedGraph, rkyv::rancor::Error>(&bytes).expect("access");
    let deserialized: Graph = rkyv::deserialize::<Graph, rkyv::rancor::Error>(archived).expect("deserialize");

    let n = &deserialized.nodes[&nid];
    assert_eq!(n.expanded_tab, 1);
    assert_eq!(n.scroll_offset_y, 45.5);
}

