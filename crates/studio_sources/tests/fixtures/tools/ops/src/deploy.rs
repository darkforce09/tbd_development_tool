use clap::Subcommand;

/// Where to deploy.
#[derive(Debug, Subcommand)]
pub enum DeployCmd {
    /// Ship the website
    Website,
    /// Ship the game server
    GameServer { region: String },
}
