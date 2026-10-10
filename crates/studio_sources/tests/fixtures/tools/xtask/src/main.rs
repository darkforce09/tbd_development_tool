mod cli;
use clap::Parser;

fn main() {
    let _ = cli::Cli::parse();
}
