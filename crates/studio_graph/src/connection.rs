use crate::model::{DataType, Graph, NodeId, PortDirection, PortId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    SelfConnection,
    NodeNotFound,
    PortNotFound,
    InvalidDirections {
        from: PortDirection,
        to: PortDirection,
    },
    IncompatibleTypes {
        from: DataType,
        to: DataType,
    },
    AlreadyConnected,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelfConnection => write!(f, "Cannot connect a node to itself"),
            NodeNotFound => write!(f, "Node not found"),
            PortNotFound => write!(f, "Port not found"),
            InvalidDirections { from, to } => {
                write!(f, "Cannot connect {:?} to {:?}", from, to)
            }
            IncompatibleTypes { from, to } => {
                write!(
                    f,
                    "Incompatible types: {} cannot connect to {}",
                    from.display_name(),
                    to.display_name()
                )
            }
            AlreadyConnected => write!(f, "Connection already exists"),
        }
    }
}

use ConnectionError::*;

/// Checks if two ports can form a valid connection in the graph.
/// Automatically handles either (output -> input) or (input -> output) order.
pub fn can_connect(
    graph: &Graph,
    source_node: NodeId,
    source_port: PortId,
    target_node: NodeId,
    target_port: PortId,
) -> Result<(NodeId, PortId, NodeId, PortId), ConnectionError> {
    if source_node == target_node {
        return Err(SelfConnection);
    }

    let p1 = graph
        .find_port(source_node, source_port)
        .ok_or(PortNotFound)?;
    let p2 = graph
        .find_port(target_node, target_port)
        .ok_or(PortNotFound)?;

    // Determine which is output and which is input
    let (out_node, out_port, in_node, in_port, out_type, in_type) =
        match (p1.direction, p2.direction) {
            (PortDirection::Output, PortDirection::Input) => (
                source_node,
                source_port,
                target_node,
                target_port,
                p1.data_type.clone(),
                p2.data_type.clone(),
            ),
            (PortDirection::Input, PortDirection::Output) => (
                target_node,
                target_port,
                source_node,
                source_port,
                p2.data_type.clone(),
                p1.data_type.clone(),
            ),
            (d1, d2) => return Err(InvalidDirections { from: d1, to: d2 }),
        };

    // Check type compatibility (exact match or convertible)
    let compatible = match (&out_type, &in_type) {
        (a, b) if a == b => true,
        (DataType::Flow, DataType::Flow) => true,
        (DataType::RustFlow, DataType::RustFlow) => true,
        (DataType::RustFlow, _) | (_, DataType::RustFlow) => true,
        (DataType::RustType(t1), DataType::RustType(t2)) => {
            t1 == t2
                || t1.trim_start_matches('&').trim_start_matches("mut ").trim()
                    == t2.trim_start_matches('&').trim_start_matches("mut ").trim()
        }
        _ => false,
    };

    if !compatible {
        return Err(IncompatibleTypes {
            from: out_type,
            to: in_type,
        });
    }

    // Check if duplicate
    let exists = graph.edges.iter().any(|e| {
        e.from_node == out_node
            && e.from_port == out_port
            && e.to_node == in_node
            && e.to_port == in_port
    });

    if exists {
        return Err(AlreadyConnected);
    }

    Ok((out_node, out_port, in_node, in_port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::create_showcase_graph;
    use crate::model::NodeArchetype;

    #[test]
    fn test_showcase_graph_integrity() {
        let graph = create_showcase_graph();
        assert_eq!(graph.nodes.len(), 5);
        assert_eq!(graph.edges.len(), 4);
    }

    #[test]
    fn test_connection_validation() {
        let mut graph = Graph::new();
        let n1 = graph.add_node(
            "Node 1",
            NodeArchetype::Ingress,
            "",
            None,
            vec![],
            vec![("AudioOut".to_string(), DataType::Audio)],
            [0.0, 0.0],
        );
        let n2 = graph.add_node(
            "Node 2",
            NodeArchetype::Compute,
            "",
            None,
            vec![("AudioIn".to_string(), DataType::Audio)],
            vec![("VisionOut".to_string(), DataType::Vision)],
            [100.0, 0.0],
        );

        let p1_out = graph.nodes[&n1].outputs[0].id;
        let p2_in = graph.nodes[&n2].inputs[0].id;
        let p2_out = graph.nodes[&n2].outputs[0].id;

        // Valid connection
        assert!(can_connect(&graph, n1, p1_out, n2, p2_in).is_ok());

        // Self connection fails
        assert_eq!(
            can_connect(&graph, n1, p1_out, n1, p1_out),
            Err(ConnectionError::SelfConnection)
        );

        // Direction mismatch (output to output)
        assert_eq!(
            can_connect(&graph, n1, p1_out, n2, p2_out),
            Err(ConnectionError::InvalidDirections {
                from: PortDirection::Output,
                to: PortDirection::Output,
            })
        );
    }

    #[test]
    fn test_fast_indices_and_isolated_counts() {
        let mut graph = Graph::new();
        assert_eq!(graph.isolated_nodes_count(), 0);

        let n1 = graph.add_node("Node A", NodeArchetype::Function, "", None, vec![], vec![("out".to_string(), DataType::Flow)], [0.0, 0.0]);
        let n2 = graph.add_node("Node B", NodeArchetype::Compute, "", None, vec![("in".to_string(), DataType::Flow)], vec![], [50.0, 50.0]);

        assert_eq!(graph.isolated_nodes_count(), 2);
        assert_eq!(graph.archetype_count(NodeArchetype::Function), 1);
        assert_eq!(graph.archetype_count(NodeArchetype::Compute), 1);

        let p1 = graph.nodes[&n1].outputs[0].id;
        let p2 = graph.nodes[&n2].inputs[0].id;

        assert!(!graph.is_port_connected(n1, p1));
        assert!(!graph.is_port_connected(n2, p2));

        let edge_id = graph.connect(n1, p1, n2, p2).expect("Should connect");
        assert!(graph.is_port_connected(n1, p1));
        assert!(graph.is_port_connected(n2, p2));
        assert_eq!(graph.isolated_nodes_count(), 0);

        graph.disconnect_edge(edge_id);
        assert!(!graph.is_port_connected(n1, p1));
        assert!(!graph.is_port_connected(n2, p2));
        assert_eq!(graph.isolated_nodes_count(), 2);
    }
}

