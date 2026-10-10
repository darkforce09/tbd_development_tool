use crate::model::{DataType, EdgeKind, Graph, NodeArchetype};

/// A small graph for tests and benchmarks: five file cards of a made-up command-line tool and the
/// four wires between them. Nothing in the app shows it; real projects come from the parser.
pub fn create_sample_graph() -> Graph {
    let mut graph = Graph::new();
    let mut card = |name: &str, x: f32, y: f32| {
        graph.add_node(
            name,
            NodeArchetype::File,
            "",
            Some("RS".to_string()),
            vec![("in".to_string(), DataType::RustFlow)],
            vec![("out".to_string(), DataType::RustFlow)],
            [x, y],
        )
    };
    let main = card("main.rs", 900.0, 260.0);
    let cli = card("cli.rs", 500.0, 100.0);
    let config = card("config.rs", 100.0, 140.0);
    let store = card("store.rs", 500.0, 420.0);
    let model = card("model.rs", 100.0, 420.0);

    for (from, to, kind, label, line) in [
        (config, cli, EdgeKind::TypeUse, "Config", 12),
        (cli, main, EdgeKind::Call, "parse_args", 4),
        (model, store, EdgeKind::Import, "model", 1),
        (store, main, EdgeKind::Call, "open", 9),
    ] {
        let out = graph.nodes[&from].outputs[0].id;
        let inp = graph.nodes[&to].inputs[0].id;
        graph.connect_labeled(from, out, to, inp, kind, Some(label.to_string()), None, Some(line));
    }
    graph
}
