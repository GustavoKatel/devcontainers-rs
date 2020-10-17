#[derive(Debug)]
pub enum Error {
    ConfigDoesNotExist(String),
    InvalidConfig(String),
    UpError(UpError),
}

#[derive(Debug)]
pub enum UpError {
    NoDevContainer,
    ContainerCreate(String),
    ApplicationSpawn(String),
    ExecCommand(String),
    ImagePull(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ConfigDoesNotExist(file) => write!(f, "Config file does not exist: {}", file),
            Error::InvalidConfig(err) => write!(f, "Config is not valid: {}", err),
            Error::UpError(err) => write!(f, "Error trying to start project: {}", err),
        }
    }
}

impl std::fmt::Display for UpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpError::NoDevContainer => {
                write!(f, "Unexpected error! No devcontainer project found!")
            }
            UpError::ContainerCreate(err) => write!(f, "Failed to create container: {}", err),
            UpError::ApplicationSpawn(err) => write!(f, "Failed to spawn application: {}", err),
            UpError::ExecCommand(err) => write!(f, "Failed to execute command: {}", err),
            UpError::ImagePull(err) => {
                write!(f, "Failed while trying to pull docker image: {}", err)
            }
        }
    }
}

impl std::convert::From<UpError> for Error {
    fn from(e: UpError) -> Self {
        Error::UpError(e)
    }
}
