#[macro_use]
extern crate log;

use clap::{App, Arg, SubCommand};
use std::path::PathBuf;
use tokio;

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

#[tokio::main]
async fn main() {
    let env = env_logger::Env::default()
        .filter_or("LOG_LEVEL", "info")
        .write_style_or("LOG_STYLE", "always");
    env_logger::init_from_env(env);

    let matches = App::new("devcontainer-rs")
        .version("0.1")
        .author("Gustavo Sampaio <gbritosampaio@gmail.com>")
        .about("An open-source runner for the devcontainer format")
        .arg(
            Arg::with_name("docker-host")
                .short("a")
                .long("host")
                .value_name("STRING")
                .help("Use the specified address to connect to docker")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("no-user-settings")
                .short("s")
                .long("no-user-settings")
                .help("Ignore global user settings")
                .takes_value(false),
        )
        .arg(
            Arg::with_name("path")
                .short("c")
                .long("path")
                .value_name("FILE")
                .help("Sets a custom cwd. The path that contains the .devcontainer folder")
                .takes_value(true),
        )
        .subcommand(
            SubCommand::with_name("up")
                .about("starts the devcontainer")
                .arg(
                    Arg::with_name("no-wait")
                        .short("d")
                        .long("no-wait")
                        .help("Do not wait for the client")
                        .takes_value(false),
                ),
        )
        .subcommand(SubCommand::with_name("down").about("stops the devcontainer"))
        .get_matches();

    let path = matches.value_of("path").map(PathBuf::from);

    let should_load_user_settings = match matches.is_present("no-user-settings") {
        true => Some(false),
        false => None,
    };

    let mut project = project::Project::new(project::ProjectOpts {
        path,
        should_load_user_settings,
        ..project::ProjectOpts::default()
    })
    .unwrap();
    project.docket_host = matches.value_of("docker-host").map(|s| s.to_string());

    if let Err(err) = project.load().await {
        panic!("Error found validating the config file: {}", err);
    }

    let res = match matches.subcommand() {
        ("up", Some(sub_matches)) => {
            let should_wait = !sub_matches.is_present("no-wait");

            project.up(should_wait).await
        }
        ("down", Some(_)) => project.down(None, false).await,
        _ => Ok(()),
    };

    res.unwrap()
}
