use clap::{Args, Parser, Subcommand};

/// Workspace tasks
#[derive(Parser)]
#[command(name = "xtask")]
pub struct Cli {
    #[command(subcommand)]
    cmd: TopCmd,
}

#[derive(Subcommand)]
enum TopCmd {
    /// Build everything
    Build,
    /// Deploy a part
    Deploy {
        #[command(subcommand)]
        target: ops::DeployCmd,
    },
    /// Database chores
    Db(DbArgs),
    /// Cache chores
    Cache {
        #[command(subcommand)]
        cmd: Option<CacheCmd>,
    },
    #[command(name = "fmt-all")]
    FormatAll,
    #[command(skip)]
    Internal,
}

#[derive(Args)]
struct DbArgs {
    #[command(subcommand)]
    cmd: DbCmd,
}

#[derive(Subcommand)]
enum DbCmd {
    /// Apply migrations
    Migrate,
    /// Drop and recreate
    Reset { force: bool },
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Show cache sizes
    Show,
    /// Clear one cache
    Clear(ClearArgs),
}

#[derive(Args)]
struct ClearArgs {
    #[command(subcommand)]
    which: Which,
}

#[derive(Subcommand)]
enum Which { Index, Layout }
