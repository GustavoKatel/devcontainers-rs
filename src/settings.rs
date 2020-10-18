use dirs;
use json5;
use serde::{de, Deserialize, Deserializer};
use std::collections::BTreeMap;
use tokio::fs;

use super::devcontainer::CommandLineVec;
use super::errors::*;

#[derive(Deserialize)]
pub struct Application {
    pub cmd: CommandLineVec,
}

#[derive(Deserialize, Default)]
pub struct Settings {
    pub application: Option<Application>,

    pub mounts: Option<Vec<String>>,

    pub envs: Option<BTreeMap<String, String>>,

    #[serde(rename = "postCreateCommand")]
    pub post_create_command: Option<CommandLineVec>,

    #[serde(rename = "postStartCommand")]
    pub post_start_command: Option<CommandLineVec>,

    #[serde(rename = "postAttachCommand")]
    pub post_attach_command: Option<CommandLineVec>,
}

impl Settings {
    pub async fn load() -> Result<Self, Error> {
        let mut settings_path = dirs::config_dir().unwrap();

        settings_path.push("devcontainer.json");

        if !settings_path.exists() {
            return Ok(Settings::default());
        }

        let contents = fs::read_to_string(settings_path)
            .await
            .map_err(|err| Error::InvalidSettings(err.to_string()))?;

        let settings: Settings =
            json5::from_str(&contents).map_err(|err| Error::InvalidSettings(err.to_string()))?;

        Ok(settings)
    }
}
