use bollard::errors::Error as DockerError;
use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("Config file does not exist: {0}")]
    ConfigDoesNotExist(String),
    #[error("Config is not valid: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    UpError(#[from] UpError),
    #[error(transparent)]
    DockerError(#[from] DockerError),
    #[error(transparent)]
    DownError(#[from] DownError),
    #[error("No devcontainer project found")]
    NoDevContainer,
    #[error("Error trying to parse settings: {0}")]
    InvalidSettings(String),
    #[error("Error trying to execute command: {0}")]
    ExecCommandError(String),
    #[error(transparent)]
    YamlError(#[from] serde_yaml::Error),
    #[error("Error trying to load settings: {0}")]
    JSON5Error(#[from] json5::Error),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error("Unexpected error: {0}")]
    Other(String),
}

#[derive(Debug, ThisError)]
pub enum UpError {
    #[error("Failed to create container: {0}")]
    ContainerCreate(String),
    #[error("Failed to spawn application: {0}")]
    ApplicationSpawn(String),
    #[error("Failed while trying to pull docker image: {0}")]
    ImagePull(DockerError),
}

#[derive(Debug, ThisError)]
pub enum DownError {}
