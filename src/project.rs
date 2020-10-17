use futures::StreamExt;
use http;
use json5;
use shiplift::{
    builder::{ContainerOptionsBuilder, ExecContainerOptions},
    rep::ContainerCreateInfo,
    tty::TtyChunk,
    Container, ContainerOptions, Docker, PullOptions,
};
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

        if let Some(pb) = path {
            dc.path =
                std::fs::canonicalize(&pb).map_err(|err| Error::InvalidConfig(err.to_string()))?;
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
        let opts = PullOptions::builder().image(image.as_str()).build();

        info!("Pulling image: {}", image);
        let mut stream = docker.images().pull(&opts);

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

    async fn docker_exec<'a>(
        &self,
        container: &'a Container<'a>,
        cmd: &CommandLineVec,
    ) -> Result<(), Error> {
        let options = ExecContainerOptions::builder()
            .cmd(cmd.to_args_vec().iter().map(|s| s.as_str()).collect())
            .attach_stdout(true)
            .attach_stderr(true)
            .build();

        let debug_output = |chunk: TtyChunk| match chunk {
            TtyChunk::StdOut(bytes) => debug!("STDOUT: {}", std::str::from_utf8(&bytes).unwrap()),
            TtyChunk::StdErr(bytes) => debug!("STDERR: {}", std::str::from_utf8(&bytes).unwrap()),
            _ => unreachable!(),
        };

        info!("Executing command in container: {}", container.id());
        debug!("Args: {:?}", cmd.to_args_vec());
        while let Some(exec_result) = container.exec(&options).next().await {
            match exec_result {
                Ok(chunk) => debug_output(chunk),
                Err(e) => {
                    error!("Error while executing command in container: {}", e);
                    return Err(Error::UpError(UpError::ExecCommand(e.to_string())));
                }
            }
        }

        Ok(())
    }

    async fn container_opts_build_ports<'a>(
        &self,
        devcontainer: &DevContainer,
        opts_ref: &'a mut ContainerOptionsBuilder,
    ) -> Result<&'a mut ContainerOptionsBuilder, Error> {
        let mut opts_ref = opts_ref;

        if let Some(app_port) = devcontainer.app_port.as_ref() {
            opts_ref = match app_port {
                AppPort::Port(p) => opts_ref.expose(*p, "tcp", *p),
                AppPort::Ports(ports) => {
                    for p in ports {
                        opts_ref = opts_ref.expose(*p, "tcp", *p);
                    }
                    opts_ref
                }
                AppPort::PortStr(p_str) => {
                    let p = p_str
                        .parse::<u32>()
                        .map_err(|err| Error::InvalidConfig(err.to_string()))?;
                    opts_ref.expose(p, "tcp", p)
                }
            };
        }
        Ok(opts_ref)
    }

    async fn container_opts_build_envs<'a>(
        &self,
        devcontainer: &DevContainer,
        opts_ref: &'a mut ContainerOptionsBuilder,
    ) -> Result<&'a mut ContainerOptionsBuilder, Error> {
        let mut opts_ref = opts_ref;

        if let Some(env_map) = devcontainer.container_env.as_ref() {
            let envs: Vec<String> = env_map
                .iter()
                .map(|(key, value)| format!("{}={}", key, value))
                .collect();
            opts_ref = opts_ref.env(envs.iter().map(|s| s.as_str()).collect());
        }

        Ok(opts_ref)
    }

    async fn container_opts_build_mounts<'a>(
        &self,
        devcontainer: &DevContainer,
        opts_ref: &'a mut ContainerOptionsBuilder,
    ) -> Result<&'a mut ContainerOptionsBuilder, Error> {
        let mut opts_ref = opts_ref;

        let wk_mount = match devcontainer.workspace_mount.as_ref() {
            None => {
                let current_dir = self.path.to_str().unwrap();
                // TODO this needs improvement: use consistency:cached
                format!("{}:/workspace", current_dir)
            }
            Some(p) => p.clone(),
        };

        let mut mounts: Vec<String> = vec![wk_mount];

        if let Some(dev_mounts) = devcontainer.mounts.as_ref() {
            mounts.extend(dev_mounts.iter().map(|s| s.clone()));
        }

        opts_ref = opts_ref.volumes(mounts.iter().map(|s| s.as_str()).collect());

        Ok(opts_ref)
    }

    async fn container_opts_build_cmd<'a>(
        &self,
        devcontainer: &DevContainer,
        opts_ref: &'a mut ContainerOptionsBuilder,
    ) -> Result<&'a mut ContainerOptionsBuilder, Error> {
        let mut opts_ref = opts_ref;

        // TODO find a way to add run args (capabilities and seccomp)
        //if let Some(args) = devcontainer.run_args.as_ref() {
        //opts_ref = opts_ref.cmd(args.iter().map(|s| s.as_str()).collect());
        //}

        if devcontainer.override_command {
            opts_ref = opts_ref.cmd(vec!["/bin/sh", "-c", "while sleep 1000; do :; done"]);
        }

        Ok(opts_ref)
    }

    async fn up_docker(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        initial_opts: ContainerOptionsBuilder,
    ) -> Result<ContainerCreateInfo, Error> {
        let mut opts = initial_opts;
        let mut opts_ref = &mut opts;

        opts_ref = self
            .container_opts_build_ports(devcontainer, opts_ref)
            .await?;

        opts_ref = self
            .container_opts_build_envs(devcontainer, opts_ref)
            .await?;

        opts_ref = self
            .container_opts_build_mounts(devcontainer, opts_ref)
            .await?;

        opts_ref = self
            .container_opts_build_cmd(devcontainer, opts_ref)
            .await?;

        let mut labels = HashMap::new();
        labels.insert("devcontainer", "true");
        opts_ref = opts_ref.labels(&labels);

        let opts_built = opts_ref.build();

        let info = docker
            .containers()
            .create(&opts_built)
            .await
            .map_err(|err| UpError::ContainerCreate(err.to_string()))?;

        Ok(info)
    }

    fn docker_format_image(&self, image: String) -> String {
        if image.contains(":") {
            return image;
        }

        format!("{}:latest", image)
    }

    async fn up_from_image<'a>(
        &self,
        docker: &'a Docker,
        devcontainer: &DevContainer,
    ) -> Result<Container<'a>, Error> {
        let image = self.docker_format_image(devcontainer.image.as_ref().unwrap().to_string());

        self.docker_pull_image(docker, image.clone()).await?;

        // TODO CHECK IF CONTAINER ALREADY EXISTS

        let opts = ContainerOptions::builder(image.as_ref());

        let info = self.up_docker(&docker, devcontainer, opts).await?;

        info!("Creating container from: {}", image);
        let container = Container::new(&docker, info.id);

        info!("Starting container");
        container
            .start()
            .await
            .map_err(|err| UpError::ContainerCreate(err.to_string()))?;

        // postCreateCommand
        if let Some(cmd) = devcontainer.post_create_command.as_ref() {
            self.docker_exec(&container, cmd).await?;
        }

        // postStartCommand
        if let Some(cmd) = devcontainer.post_start_command.as_ref() {
            self.docker_exec(&container, cmd).await?;
        }

        Ok(container)
    }

    async fn up_from_build<'a>(
        &self,
        docker: &'a Docker,
        devcontainer: &DevContainer,
    ) -> Result<Container<'a>, Error> {
        todo!()
    }

    async fn up_from_compose<'a>(
        &self,
        docker: &'a Docker,
        devcontainer: &DevContainer,
    ) -> Result<Container<'a>, Error> {
        todo!()
    }

    pub async fn up(&self, should_wait: bool) -> Result<(), Error> {
        let devcontainer = self.devcontainer.as_ref().ok_or(UpError::NoDevContainer)?;

        let docker = match self.docket_host.as_ref() {
            None => Docker::new(),
            Some(h) => {
                let host = h
                    .parse::<http::uri::Uri>()
                    .map_err(|err| UpError::ContainerCreate(err.to_string()))?;
                Docker::host(host)
            }
        };

        info!("Starting containers");

        let container = if devcontainer.image.is_some() {
            self.up_from_image(&docker, &devcontainer).await?
        } else if devcontainer.build.is_some() {
            self.up_from_build(&docker, &devcontainer).await?
        } else {
            self.up_from_compose(&docker, &devcontainer).await?
        };

        info!("Containers are ready: {}", container.id());

        let child = if devcontainer.application.is_some() {
            Some(self.spawn_application(devcontainer).await?)
        } else {
            None
        };

        if !should_wait {
            return Ok(());
        }

        let signal_stream = signal::ctrl_c();

        if let Some(child) = child {
            info!("Waiting for application");
            tokio::select! {
                child_res = child => {
                    if let Err(err) = child_res {
                        return Err(Error::UpError(UpError::ApplicationSpawn(err.to_string())));
                    }
                    info!("Application has finished. Closing down");
                },
                _ = container.wait() => {
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
            _ = container.wait() => {
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
