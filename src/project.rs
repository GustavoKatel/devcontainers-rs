use bollard::{
    container::{self, StartContainerOptions, CreateContainerOptions, ListContainersOptions},
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    image::CreateImageOptions,
    Docker, API_DEFAULT_VERSION,
};
use futures::StreamExt;
use json5;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tokio::process::{Child, Command};
use tokio::signal;

use crate::devcontainer::*;
use crate::errors::*;

pub struct Project {
    pub path: PathBuf,
    pub filename: String,

    pub docket_host: Option<String>,

    pub devcontainer: Option<DevContainer>,
}

impl std::default::Default for Project {
    fn default() -> Self {
        let path = std::env::current_dir().unwrap();
        Project {
            filename: "devcontainer.json".to_string(),
            path,

            docket_host: None,

            devcontainer: None,
        }
    }
}

impl Project {
    pub fn new(path: Option<PathBuf>, filename: Option<String>) -> Result<Self, Error> {
        let mut dc = Self::default();
        let mut path = if let Some(pb) = path {
            pb
        } else {
            PathBuf::new()
        };

        path.canonicalize().map_err(|err| Error::InvalidConfig(err.to_string()))?;

        for ancestor in path.ancestors() {
            if ancestor.join(".devcontainer").exists() {
                dc.path = ancestor.to_path_buf();
            } 
        }

        if let Some(f) = filename {
            dc.filename = f;
        }

        Ok(dc)
    }

    pub async fn load(&mut self) -> Result<(), Error> {
        let mut filename = self.path.clone();
        filename.push(".devcontainer");
        filename.push(self.filename.clone());

        info!("Loading project: {}", self.path.to_str().unwrap());
        info!("devcontainer.json: {}", filename.to_str().unwrap());

        if !filename.exists() {
            return Err(Error::ConfigDoesNotExist(
                filename.to_str().unwrap().to_string(),
            ));
        }

        let contents = fs::read_to_string(filename.as_path())
            .await
            .map_err(|err| Error::InvalidConfig(err.to_string()))?;

        let devcontainer: DevContainer =
            json5::from_str(&contents).map_err(|err| Error::InvalidConfig(err.to_string()))?;

        if let Err(err) = devcontainer.validate() {
            return Err(err);
        }

        self.devcontainer = Some(devcontainer);

        Ok(())
    }

    async fn spawn_application(&self, devcontainer: &DevContainer) -> Result<Child, Error> {
        info!("Found application settings. Spawning");
        let application = devcontainer.application.as_ref().unwrap();

        let args = application.cmd.to_args_vec();

        let mut builder = &mut Command::new(args[0].clone());
        builder = builder.args(args.iter().skip(1));

        let child = builder
            .spawn()
            .map_err(|err| UpError::ApplicationSpawn(err.to_string()))?;
        Ok(child)
    }

    async fn docker_pull_image(&self, docker: &Docker, image: String) -> Result<(), UpError> {
        info!("Pulling image: {}", image);
        let options = Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        });

        let mut stream = docker.create_image(options, None, None);

        while let Some(pull_result) = stream.next().await {
            match pull_result {
                Ok(output) => {
                    debug!("Pull output: {:?}", output);
                }
                Err(e) => {
                    error!("Pull error: {}", e);
                    return Err(UpError::ImagePull(e.to_string()));
                }
            }
        }

        info!("Pulling image: done");

        Ok(())
    }

    async fn docker_exec(
        &self,
        docker: &Docker,
        id: String,
        cmd: &CommandLineVec,
    ) -> Result<(), Error> {
        let options = CreateExecOptions {
            cmd: Some(cmd.to_args_vec()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        let exec = docker.create_exec(id.as_str(), options).await?;

        let mut stream = docker.start_exec(exec.id.as_str(), None::<StartExecOptions>);

        info!("Executing command in container: {}", id);
        debug!("Args: {:?}", cmd.to_args_vec());
        while let Some(exec_result) = stream.next().await {
            match exec_result? {
                StartExecResults::Attached { log: log } => match log {
                    container::LogOutput::StdOut { message: bytes } => {
                        debug!("STDOUT: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::StdErr { message: bytes } => {
                        debug!("STDERR: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::Console { message: bytes } => {
                        debug!("CONSOLE: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::StdIn { message: bytes } => unreachable!(),
                },
                StartExecResults::Detached => { /*nothing to do here*/ }
            }
        }

        Ok(())
    }

    async fn container_opts_build_ports(
        &self,
        devcontainer: &DevContainer,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        let mut ports_to_expose: HashMap<String, HashMap<(), ()>> = HashMap::new();

        if let Some(app_port) = devcontainer.app_port.as_ref() {
            match app_port {
                AppPort::Port(p) => {
                    ports_to_expose.insert(p.to_string(), HashMap::new());
                }
                AppPort::Ports(ports) => {
                    for p in ports {
                        ports_to_expose.insert(format!("{}/tcp", p), HashMap::new());
                    }
                }
                AppPort::PortStr(p_str) => {
                    let p = p_str
                        .parse::<u32>()
                        .map_err(|err| Error::InvalidConfig(err.to_string()))?;
                    ports_to_expose.insert(format!("{}/tcp", p), HashMap::new());
                }
            };
        }

        config.exposed_ports = Some(ports_to_expose);

        Ok(())
    }

    async fn container_opts_build_envs(
        &self,
        devcontainer: &DevContainer,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        if let Some(env_map) = devcontainer.container_env.as_ref() {
            let envs: Vec<String> = env_map
                .iter()
                .map(|(key, value)| format!("{}={}", key, value))
                .collect();
            config.env = Some(envs);
        }

        Ok(())
    }

    async fn container_opts_build_mounts(
        &self,
        devcontainer: &DevContainer,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        let mut mounts: HashMap<String, HashMap<(), ()>> = HashMap::new();

        let wk_mount = match devcontainer.workspace_mount.as_ref() {
            None => {
                let current_dir = self.path.to_str().unwrap();
                // TODO this needs improvement: use consistency:cached
                //format!("{}:/workspace", current_dir)
                format!(
                    "source={},target=/workspace,type=bind,consistency=cached",
                    current_dir
                )
            }
            Some(p) => p.clone(),
        };

        mounts.insert(wk_mount, HashMap::new());

        if let Some(dev_mounts) = devcontainer.mounts.as_ref() {
            for m in dev_mounts.iter() {
                mounts.insert(m.clone(), HashMap::new());
            }
        }

        config.volumes = Some(mounts);

        Ok(())
    }

    async fn container_opts_build_cmd(
        &self,
        devcontainer: &DevContainer,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        // TODO find a way to add run args (capabilities and seccomp)
        //if let Some(args) = devcontainer.run_args.as_ref() {
        //opts_ref = opts_ref.cmd(args.iter().map(|s| s.as_str()).collect());
        //}

        if devcontainer.override_command {
            config.cmd = Some(
                vec!["/bin/sh", "-c", "while sleep 1000; do :; done"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        }

        Ok(())
    }

    async fn up_docker(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        image: String,
    ) -> Result<String, Error> {
        let mut config: container::Config<String> = container::Config {
            image: Some(image.clone()),
            ..Default::default()
        };

        self.container_opts_build_ports(devcontainer, &mut config)
            .await?;

        self.container_opts_build_envs(devcontainer, &mut config)
            .await?;

        self.container_opts_build_mounts(devcontainer, &mut config)
            .await?;

        self.container_opts_build_cmd(devcontainer, &mut config)
            .await?;

        let mut labels = HashMap::new();
        labels.insert("devcontainer".to_string(), "true".to_string());

        config.labels = Some(labels);
        let mut container_options: Option<CreateContainerOptions<String>> = None;

        if let Some(filename) = self.path.file_name() {
            if let Some(filename) = filename.to_str() {
                let image_name: &str = image.split(':').next().unwrap();

                // Use unique id to avoid collision with existing containers
                for id in 1..20 {
                    let name = format!("{}_devcontainer_{}_{}", filename, image_name, id);

                    let mut filters = HashMap::new();
                    filters.insert("name", vec![name.as_str()]);

                    let options = Some(ListContainersOptions{
                        all: true,
                        filters: filters,
                        ..std::default::Default::default()
                    });

                    // Check if an existing container has this name
                    if let Ok(containers) = docker.list_containers(options).await {
                        if containers.len() > 0 {
                            continue;
                        }
                    }

                    container_options = Some(CreateContainerOptions{
                        name
                    });

                    break;
                }

            }
        }

        let info = docker
            .create_container::<String, String>(container_options, config)
            .await?;

        Ok(info.id)
    }

    fn docker_format_image(&self, image: String) -> String {
        if image.contains(":") {
            return image;
        }

        format!("{}:latest", image)
    }

    async fn up_from_image(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
    ) -> Result<String, Error> {
        let image = self.docker_format_image(devcontainer.image.as_ref().unwrap().to_string());

        self.docker_pull_image(docker, image.clone()).await?;

        info!("Creating container from: {}", image);
        let id = self.up_docker(&docker, devcontainer, image).await?;

        info!("Starting container");
        docker
            .start_container(id.as_str(), None::<StartContainerOptions<String>>)
            .await?;

        // postCreateCommand
        if let Some(cmd) = devcontainer.post_create_command.as_ref() {
            self.docker_exec(docker, id.clone(), cmd).await?;
        }

        // postStartCommand
        if let Some(cmd) = devcontainer.post_start_command.as_ref() {
            self.docker_exec(docker, id.clone(), cmd).await?;
        }

        Ok(id)
    }

    async fn up_from_build<'a>(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
    ) -> Result<String, Error> {
        todo!()
    }

    async fn up_from_compose<'a>(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
    ) -> Result<String, Error> {
        todo!()
    }

    async fn create_docker_client(&self) -> Result<Docker, Error> {
        let docker = match self.docket_host.as_ref() {
            None => Docker::connect_with_local_defaults()?,
            Some(h) => {
                let host = h.as_str();
                Docker::connect_with_http(host, 60, API_DEFAULT_VERSION)?
            }
        };

        Ok(docker)
    }

    pub async fn up(&self, should_wait: bool) -> Result<(), Error> {
        let devcontainer = self.devcontainer.as_ref().ok_or(UpError::NoDevContainer)?;

        let docker = self.create_docker_client().await?;

        info!("Starting containers");

        let container_id = if devcontainer.image.is_some() {
            self.up_from_image(&docker, &devcontainer).await?
        } else if devcontainer.build.is_some() {
            self.up_from_build(&docker, &devcontainer).await?
        } else {
            self.up_from_compose(&docker, &devcontainer).await?
        };

        info!("Containers are ready: {}", container_id);

        let child = if devcontainer.application.is_some() {
            Some(self.spawn_application(devcontainer).await?)
        } else {
            None
        };

        if !should_wait {
            return Ok(());
        }

        let signal_stream = signal::ctrl_c();

        let mut container_wait_stream = docker.wait_container(
            container_id.as_str(),
            None::<container::WaitContainerOptions<String>>,
        );

        if let Some(child) = child {
            info!("Waiting for application");
            tokio::select! {
                child_res = child => {
                    if let Err(err) = child_res {
                        return Err(Error::UpError(UpError::ApplicationSpawn(err.to_string())));
                    }
                    info!("Application has finished. Closing down");
                },
                _ = &mut container_wait_stream.next() => {
                    warn!("Container has finished! Restart required");
                    return Ok(());
                },
                _ = signal_stream => {
                    info!("CTRL+C: Finishing now");
                }
            };
            return self.down(true).await;
        }

        let should_go_down = tokio::select! {
            _ = &mut container_wait_stream.next() => {
                warn!("Container has finished! Nothing to do now. Closing down.");
                false
            }
            _ = signal_stream  => {
                info!("CTRL+C: Finishing now");
                true
            }
        };

        if !should_go_down {
            return Ok(());
        }

        self.down(true).await
    }

    pub async fn down(&self, from_up: bool) -> Result<(), Error> {
        info!("Shutting down containers");
        Ok(())
    }
}
