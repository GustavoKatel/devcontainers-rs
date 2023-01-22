use dirs;
use json5;
use serde::Deserialize;
use serde_yaml;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use super::devcontainer::CommandLineVec;
use super::errors::*;
use super::settings_compose_model::*;

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

    #[serde(rename = "forwardPorts")]
    pub forward_ports: Option<Vec<i32>>,
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

        let settings: Settings = json5::from_str(&contents)?;

        Ok(settings)
    }

    pub async fn generate_compose_override(
        &self,
        service_name: String,
        version: String,
        envs: Option<HashMap<String, String>>,
        ext_ports: Option<Vec<i32>>,
    ) -> Result<PathBuf, Error> {
        let mut envs = envs.unwrap_or(HashMap::new());

        if let Some(settings_envs) = self.envs.as_ref() {
            for (key, value) in settings_envs.iter() {
                envs.insert(key.clone(), value.clone());
            }
        }

        let mut ports = self
            .forward_ports
            .clone()
            .map(|ports| ports.iter().map(|p| format!("{}:{}", p, p)).collect())
            .unwrap_or(vec![]);

        if let Some(ep) = ext_ports {
            for port in ep {
                ports.push(format!("{}:{}", port, port));
            }
        }

        let service = Service {
            ports: Some(ports),
            volumes: self.mounts.clone(),
            environment: Some(envs),
            ..Service::default()
        };

        let mut services = HashMap::new();
        services.insert(service_name.clone(), service);

        let compose_model = SettingsComposeModel {
            version,
            services,
            ..SettingsComposeModel::default()
        };

        let mut path = std::env::temp_dir();
        path.push(format!("{}-compose.yml", service_name));

        let mut file = tokio::fs::File::create(&path).await?;

        let data = serde_yaml::to_string(&compose_model)?;

        file.write_all(data.as_bytes()).await?;

        Ok(path)
    }
}
