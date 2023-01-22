use clap::{Parser, Subcommand};

use std::path::PathBuf;
use tokio;

mod utils;

mod mount_from_str;
#[cfg(test)]
mod mount_from_str_tests;

mod devcontainer;
#[cfg(test)]
mod devcontainer_tests;

mod settings;
mod settings_compose_model;

mod project;
#[cfg(test)]
mod project_tests;

mod errors;

#[derive(Parser)]
#[command(author = "Gustavo Sampaio <devcontainer-rs@gsampaio.dev>")]
#[command(version, about = "devcontainer.json parser and executor", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(
        long,
        short = 'a',
        help = "Use the specified address to connect to docker"
    )]
    docker_host: Option<String>,

    #[arg(
        long = "no-user-settings",
        short = 's',
        help = "Ignore global user settings",
        default_value = "false"
    )]
    should_load_user_settings: bool,

    #[arg(
        long = "path",
        short = 'c',
        help = "Sets a custom cwd. The path that contains the .devcontainer folder. Defaults to current directory"
    )]
    path: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "starts the devcontainer")]
    Up {
        #[arg(long = "no-wait", short = 'd', help = "Do not wait for the client")]
        no_wait: bool,
    },
    #[command(about = "stops the devcontainer")]
    Down {},
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let path = match cli.path {
        Some(p) => PathBuf::from(p),
        None => PathBuf::new(),
    };

    let mut project = project::Project::new(project::ProjectOpts {
        path: Some(path),
        should_load_user_settings: Some(cli.should_load_user_settings),
        docker_host: cli.docker_host,
        ..project::ProjectOpts::default()
    })?;

    project.load().await?;

    match &cli.command {
        Commands::Up { no_wait } => project.up(!no_wait).await,
        Commands::Down {} => project.down(None, false).await,
    }
}
