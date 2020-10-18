use serde::{de, Deserialize, Deserializer};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::errors::*;

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Default)]
pub struct DevContainer {
    pub name: Option<String>,

    pub image: Option<String>,

    pub build: Option<BuildOpts>,

    #[serde(rename = "appPort")]
    pub app_port: Option<AppPort>,

    #[serde(rename = "containerEnv")]
    pub container_env: Option<BTreeMap<String, String>>,

    #[serde(rename = "remoteEnv")]
    pub remote_env: Option<BTreeMap<String, String>>,

    #[serde(rename = "containerUser")]
    pub container_user: Option<String>,

    #[serde(rename = "remoteUser")]
    pub remote_user: Option<String>,

    #[serde(default, rename = "updateRemoteUserUID")]
    pub update_remote_user_uid: bool,

    pub mounts: Option<Vec<String>>,

    #[serde(rename = "workspaceMount")]
    pub workspace_mount: Option<String>,

    #[serde(rename = "runArgs")]
    pub run_args: Option<Vec<String>>,

    #[serde(rename = "overrideCommand", default = "default_true")]
    pub override_command: bool,

    #[serde(rename = "shutdownAction")]
    pub shutdown_action: Option<ShutdownAction>,

    // Docker compose stuff
    #[serde(rename = "dockerComposeFile")]
    pub docker_compose_file: Option<DockerComposeFile>,

    pub service: Option<String>,

    #[serde(rename = "runServices")]
    pub run_services: Option<Vec<String>>,

    #[serde(rename = "forwardPorts")]
    pub forward_ports: Option<Vec<i32>>,

    #[serde(rename = "postCreateCommand")]
    pub post_create_command: Option<CommandLineVec>,

    #[serde(rename = "postStartCommand")]
    pub post_start_command: Option<CommandLineVec>,

    #[serde(rename = "postAttachCommand")]
    pub post_attach_command: Option<CommandLineVec>,

    #[serde(rename = "initializeCommand")]
    pub initialize_command: Option<CommandLineVec>,

    #[serde(rename = "devPort", default)]
    pub dev_port: i32,
}

#[derive(Deserialize)]
pub struct BuildOpts {
    #[serde(alias = "dockerFile")]
    pub dockerfile: String,

    pub context: Option<String>,

    pub args: Option<BTreeMap<String, String>>,

    pub target: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum AppPort {
    Port(u32),
    Ports(Vec<u32>),
    PortStr(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum DockerComposeFile {
    File(String),
    Files(Vec<String>),
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandLineVec {
    Line(String),
    Args(Vec<String>),
}

#[derive(Debug, PartialEq)]
pub enum ShutdownAction {
    None,
    StopContainer,
    StopCompose,
}

// Specify which mode should this devcontainer operate on
pub enum Mode {
    Image,
    Build,
    Compose,
}

impl<'de> Deserialize<'de> for ShutdownAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.to_lowercase();
        let state = match s.as_str() {
            "none" => ShutdownAction::None,
            "stopcontainer" => ShutdownAction::StopContainer,
            "stopcompose" => ShutdownAction::StopCompose,
            other => {
                return Err(de::Error::custom(format!(
                    "Invalid shutdown action '{}'",
                    other
                )));
            }
        };
        Ok(state)
    }
}

impl DevContainer {
    pub fn get_mode(&self) -> Mode {
        if self.image.is_some() {
            Mode::Image
        } else if self.build.is_some() {
            Mode::Build
        } else {
            Mode::Compose
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        // image conflicts with docker_compose_file
        let sources = vec![
            self.image.is_some(),
            self.docker_compose_file.is_some(),
            self.build.is_some(),
        ];
        if sources.iter().filter(|v| **v).count() > 1 {
            return Err(Error::InvalidConfig(
                "Please specify only one of: image, dockerComposeFile or build".to_string(),
            ));
        }
        if self.image.is_none() && self.docker_compose_file.is_none() && self.build.is_none() {
            return Err(Error::InvalidConfig(
                "Please specify at least one of: image, dockerComposeFile or build".to_string(),
            ));
        }

        if let Some(img) = self.image.as_ref() {
            if img.trim().is_empty() {
                return Err(Error::InvalidConfig(format!("Invalid image: '{}'", img)));
            }
        }

        if let Some(opts) = self.build.as_ref() {
            if opts.dockerfile.trim().is_empty() {
                return Err(Error::InvalidConfig(format!(
                    "Invalid docker file: '{}'",
                    opts.dockerfile
                )));
            }
        }

        if let Some(compose) = self.docker_compose_file.as_ref() {
            if match &compose {
                DockerComposeFile::File(dcf) => dcf.trim().is_empty(),
                DockerComposeFile::Files(v) => v.is_empty(),
            } {
                return Err(Error::InvalidConfig(
                    "Invalid docker-compose file".to_string(),
                ));
            }

            if match self.service.as_ref() {
                None => true,
                Some(s) if s.is_empty() => true,
                _ => false,
            } {
                return Err(Error::InvalidConfig("Invalid service!".to_string()));
            }
        }

        Ok(())
    }

    pub fn get_name(&self, path: &PathBuf) -> String {
        self.name
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or(path.file_name().unwrap().to_string_lossy().to_string())
    }
}

impl CommandLineVec {
    pub fn to_args_vec(&self) -> Vec<String> {
        match self {
            CommandLineVec::Line(line) => vec![line.clone()],
            CommandLineVec::Args(args) => args.clone(),
        }
    }
}
