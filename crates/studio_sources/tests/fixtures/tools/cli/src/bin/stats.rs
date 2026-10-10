use clap::Parser;

#[derive(Parser)]
#[command(about = "Print project stats")]
struct Stats {
    #[arg(long)]
    json: bool,
}

fn main() {
    let _ = Stats::parse();
}
