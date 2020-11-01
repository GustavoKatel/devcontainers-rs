use bollard::{
    container::{
        self, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
        StopContainerOptions,
    },
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    image::{BuildImageOptions, CreateImageOptions},
    service::{ContainerSummaryInner, HostConfig, Mount, PortBinding},
    Docker, API_DEFAULT_VERSION,
};
use crypto::digest::Digest;
use crypto::sha1::Sha1;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::StreamExt;
use json5;
use serde_yaml;
use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use tokio::fs;
use tokio::process::{Child, Command};
use tokio::signal;

use crate::devcontainer::*;
use crate::errors::*;
use crate::mount_from_str::*;
use crate::settings::*;
use crate::settings_compose_model::*;

#[derive(Debug)]
pub enum CommandHook {
    PostCreate,
    PostStart,
    PostAttach,
}

struct Context {
    pub application_port: Option<u16>,
    pub project_name: String,
}

pub struct Project {
    pub path: PathBuf,
    pub filename: String,

    pub docket_host: Option<String>,

    pub devcontainer: Option<DevContainer>,

    pub settings: Option<Settings>,

    pub opts: ProjectOpts,
}

impl std::default::Default for Project {
    fn default() -> Self {
        let path = std::env::current_dir().unwrap();
        Project {
            filename: "devcontainer.json".to_string(),
            path,

            docket_host: None,

            devcontainer: None,

            settings: None,

            opts: ProjectOpts::default(),
        }
    }
}

#[derive(Default)]
pub struct ProjectOpts {
    pub path: Option<PathBuf>,
    pub filename: Option<String>,
    pub should_load_user_settings: Option<bool>,
}

impl Project {
    pub fn new(opts: ProjectOpts) -> Result<Self, Error> {
        let mut dc = Self::default();
        if let Some(pb) = opts.path.as_ref() {
            pb.canonicalize()
                .map_err(|err| Error::InvalidConfig(err.to_string()))?;
            dc.path = pb.clone();
        }

        for ancestor in dc.path.clone().ancestors() {
            if ancestor.join(".devcontainer").exists() {
                dc.path = ancestor
                    .to_path_buf()
                    .canonicalize()
                    .map_err(|err| Error::InvalidConfig(err.to_string()))?;
            }
        }

        if let Some(f) = opts.filename.clone() {
            dc.filename = f;
        }

        dc.opts = opts;

        Ok(dc)
    }

    fn get_devcontainer_folder(&self) -> PathBuf {
        let mut path = self.path.clone();
        path.push(".devcontainer");

        path
    }

    pub async fn load(&mut self) -> Result<(), Error> {
        self.settings = match self.opts.should_load_user_settings.as_ref() {
            Some(false) => {
                warn!("Ignoring user settings because of -s");
                Some(Settings::default())
            }
            _ => Some(Settings::load().await?),
        };

        let mut filename = self.get_devcontainer_folder();
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

    fn get_devcontainer_envs(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
    ) -> HashMap<String, String> {
        let mut envs = HashMap::new();

        envs.insert(
            "DEVCONTAINER_PROJECT".to_string(),
            devcontainer.get_name(&self.path),
        );

        if let Some(port) = ctx.application_port.as_ref() {
            envs.insert(
                "DEVCONTAINER_APPLICATION_PORT".to_string(),
                format!("{}", port),
            );
        }

        envs
    }

    async fn spawn_application(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
    ) -> Result<Child, Error> {
        info!("Found application settings. Spawning");
        let application = self
            .settings
            .as_ref()
            .unwrap()
            .application
            .as_ref()
            .unwrap();

        let args = application.cmd.to_args_vec();

        let mut builder = &mut Command::new(args[0].clone());
        builder = builder.args(args.iter().skip(1));

        if let Some(remote_envs) = devcontainer.remote_env.as_ref() {
            builder.envs(remote_envs);
        }

        let devcontainer_envs = self.get_devcontainer_envs(devcontainer, ctx);
        debug!("{:?}", devcontainer_envs);

        builder.envs(devcontainer_envs);

        let child = builder
            .spawn()
            .map_err(|err| UpError::ApplicationSpawn(err.to_string()))?;
        Ok(child)
    }

    async fn docker_build_image(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
    ) -> Result<String, UpError> {
        let devcontainer_dir = self.get_devcontainer_folder();

        let dockerfile = devcontainer.build.as_ref().unwrap().dockerfile.clone();
        let mut file = File::open(devcontainer_dir.join(dockerfile.clone())).unwrap();
        let mut contents = String::new();
        let mut hasher = Sha1::new();
        file.read_to_string(&mut contents).unwrap();
        hasher.input_str(&contents);
        let image_name = format!("devcontainer_{}", &hasher.result_str()[0..10]);
        info!("Building image: {}", image_name);

        // API reads the Dockerfile from a tarball
        let enc = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(enc);
        tar.append_dir_all("devcontainer/", devcontainer_dir)
            .unwrap();
        let dockerfile_path: PathBuf = ["devcontainer", &dockerfile].iter().collect();

        let options = BuildImageOptions {
            dockerfile: dockerfile_path.to_str().unwrap(),
            t: &image_name.clone(),
            rm: true,
            ..std::default::Default::default()
        };

        let mut stream = docker.build_image(
            options,
            None,
            Some(tar.into_inner().unwrap().finish().unwrap().into()),
        );

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

        info!("Building image: done");

        Ok(image_name)
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
        info!("Executing command in container: {}", id);

        let options = CreateExecOptions {
            cmd: Some(cmd.to_args_vec()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        let exec = docker.create_exec(id.as_str(), options).await?;

        let mut stream = docker.start_exec(exec.id.as_str(), None::<StartExecOptions>);

        debug!("Args: {:?}", cmd.to_args_vec());
        while let Some(exec_result) = stream.next().await {
            match exec_result? {
                StartExecResults::Attached { log } => match log {
                    container::LogOutput::StdOut { message: bytes } => {
                        debug!("STDOUT: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::StdErr { message: bytes } => {
                        debug!("STDERR: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::Console { message: bytes } => {
                        debug!("CONSOLE: {}", std::str::from_utf8(&bytes).unwrap())
                    }
                    container::LogOutput::StdIn { message: _ } => unreachable!(),
                },
                StartExecResults::Detached => { /*nothing to do here*/ }
            }
        }

        let inspect = docker.inspect_exec(&exec.id).await?;
        if let Some(exit_code) = inspect.exit_code.as_ref() {
            if *exit_code != 0 {
                return Err(Error::ExecCommandError(format!("Exit code: {}", exit_code)));
            }
        }

        Ok(())
    }

    async fn run_hook(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        container_id: String,
        hook: CommandHook,
    ) -> Result<(), Error> {
        let cmd_st = match hook {
            CommandHook::PostCreate => devcontainer.post_create_command.as_ref(),
            CommandHook::PostStart => devcontainer.post_start_command.as_ref(),
            CommandHook::PostAttach => devcontainer.post_attach_command.as_ref(),
        };

        if let Some(cmd) = cmd_st {
            info!("Executing hook: {:?}", hook);
            self.docker_exec(docker, container_id.clone(), cmd).await?;
        }

        // user hooks
        let cmd_st = match hook {
            CommandHook::PostCreate => self.settings.as_ref().unwrap().post_create_command.as_ref(),
            CommandHook::PostStart => self.settings.as_ref().unwrap().post_start_command.as_ref(),
            CommandHook::PostAttach => self.settings.as_ref().unwrap().post_attach_command.as_ref(),
        };

        if let Some(cmd) = cmd_st {
            info!("Executing user hook: {:?}", hook);
            return self.docker_exec(docker, container_id, cmd).await;
        }

        Ok(())
    }

    async fn container_opts_build_ports(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        let mut ports_exposed: HashMap<String, HashMap<(), ()>> = HashMap::new();

        let mut host_config = match config.host_config.clone() {
            Some(hc) => hc,
            None => HostConfig::default(),
        };

        let mut port_bindings = match host_config.port_bindings.clone() {
            Some(m) => m,
            None => HashMap::new(),
        };

        if let Some(app_port) = devcontainer.app_port.as_ref() {
            match app_port {
                AppPort::Port(p) => {
                    port_bindings.insert(
                        format!("{}/tcp", p),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".to_string()),
                            host_port: Some(format!("{}", p)),
                        }]),
                    );
                    ports_exposed.insert(format!("{}/tcp", p), HashMap::new());
                }
                AppPort::Ports(ports) => {
                    for p in ports {
                        port_bindings.insert(
                            format!("{}/tcp", p),
                            Some(vec![PortBinding {
                                host_ip: Some(String::from("0.0.0.0")),
                                host_port: Some(format!("{}", p)),
                            }]),
                        );
                        ports_exposed.insert(format!("{}/tcp", p), HashMap::new());
                    }
                }
                AppPort::PortStr(p_str) => {
                    port_bindings.insert(
                        format!("{}/tcp", p_str),
                        Some(vec![PortBinding {
                            host_ip: Some(String::from("0.0.0.0")),
                            host_port: Some(p_str.clone()),
                        }]),
                    );
                    ports_exposed.insert(format!("{}/tcp", p_str), HashMap::new());
                }
            };
        }

        if let Some(forward_ports) = devcontainer.forward_ports.as_ref() {
            for port in forward_ports {
                port_bindings.insert(
                    format!("{}/tcp", port),
                    Some(vec![PortBinding {
                        host_ip: Some(String::from("0.0.0.0")),
                        host_port: Some(format!("{}", port)),
                    }]),
                );
                ports_exposed.insert(format!("{}/tcp", port), HashMap::new());
            }
        }

        // user ports
        if let Some(forward_ports) = self.settings.as_ref().unwrap().forward_ports.as_ref() {
            for port in forward_ports {
                port_bindings.insert(
                    format!("{}/tcp", port),
                    Some(vec![PortBinding {
                        host_ip: Some(String::from("0.0.0.0")),
                        host_port: Some(format!("{}", port)),
                    }]),
                );
                ports_exposed.insert(format!("{}/tcp", port), HashMap::new());
            }
        }

        // application_port
        if let Some(port) = ctx.application_port.as_ref() {
            port_bindings.insert(
                format!("{}/tcp", port),
                Some(vec![PortBinding {
                    host_ip: Some(String::from("0.0.0.0")),
                    host_port: Some(format!("{}", port)),
                }]),
            );
            ports_exposed.insert(format!("{}/tcp", port), HashMap::new());
        }

        host_config.port_bindings = Some(port_bindings);
        config.host_config = Some(host_config);

        config.exposed_ports = Some(ports_exposed);

        Ok(())
    }

    async fn container_opts_build_envs(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        let mut envs: Vec<String> = self
            .get_devcontainer_envs(devcontainer, ctx)
            .iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect();

        if let Some(env_map) = devcontainer.container_env.as_ref() {
            envs.extend(
                env_map
                    .iter()
                    .map(|(key, value)| format!("{}={}", key, value))
                    .collect::<Vec<String>>(),
            );
        };

        if let Some(env_map) = self.settings.as_ref().unwrap().envs.as_ref() {
            envs.extend(
                env_map
                    .iter()
                    .map(|(key, value)| format!("{}={}", key, value)),
            )
        }

        config.env = Some(envs);

        Ok(())
    }

    async fn container_opts_build_mounts(
        &self,
        devcontainer: &DevContainer,
        config: &mut container::Config<String>,
    ) -> Result<(), Error> {
        let mut host_config = match config.host_config.clone() {
            Some(hc) => hc,
            None => HostConfig::default(),
        };

        let mut mounts = match host_config.mounts.clone() {
            Some(m) => m,
            None => vec![],
        };

        let wk_mount = match devcontainer.workspace_mount.as_ref() {
            None => {
                let current_dir = self.path.to_str().unwrap();
                debug!(
                    "Mounting default workspace folder: {} to /workspace",
                    current_dir
                );
                Mount::parse_from_str(
                    format!(
                        "source={},target=/workspace,type=bind,consistency=cached",
                        current_dir,
                    )
                    .as_str(),
                )?
            }
            Some(p) => Mount::parse_from_str(p.as_str())?,
        };

        mounts.push(wk_mount);

        if let Some(dev_mounts) = devcontainer.mounts.as_ref() {
            for m in dev_mounts.iter() {
                mounts.push(Mount::parse_from_str(m.as_str())?);
            }
        }

        if let Some(user_mounts) = self.settings.as_ref().unwrap().mounts.as_ref() {
            for m in user_mounts.iter() {
                debug!("Adding user mount: {}", m);
                mounts.push(Mount::parse_from_str(m.as_str())?);
            }
        }

        host_config.mounts = Some(mounts);
        config.host_config = Some(host_config);

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

    async fn get_container_from_filters(
        &self,
        docker: &Docker,
        filters: &HashMap<&str, Vec<&str>>,
    ) -> Result<Option<ContainerSummaryInner>, Error> {
        let options = Some(ListContainersOptions {
            all: true,
            filters: filters.clone(),
            ..Default::default()
        });

        let result = docker.list_containers(options).await?;

        if result.len() > 0 {
            return Ok(Some(result[0].clone()));
        }

        Ok(None)
    }

    async fn check_is_container_running_from_name(
        &self,
        docker: &Docker,
        name: String,
    ) -> Result<Option<ContainerSummaryInner>, Error> {
        let label_name: String = format!("devcontainer_name={}", name);

        let mut filters = HashMap::new();
        filters.insert("label", vec!["devcontainer=true", label_name.as_str()]);

        self.get_container_from_filters(docker, &filters).await
    }

    async fn get_application_port(
        &self,
        stat: Option<&ContainerSummaryInner>,
    ) -> Result<u16, Error> {
        if let Some(stat) = stat {
            if let Some(labels) = stat.labels.as_ref() {
                for (key, _) in labels.iter() {
                    if key.starts_with("devcontainer_application_port=") {
                        let application_port = key.replace("devcontainer_application_port=", "");
                        let application_port: u16 = application_port.parse().map_err(|err| {
                            Error::Other(format!(
                                "Could not parse application port from container: {}",
                                err
                            ))
                        })?;

                        return Ok(application_port);
                    }
                }
            }
        }

        let application_port = match crate::utils::request_open_port().await {
            None => {
                return Err(Error::Other(
                    "Could select an available port for application".to_string(),
                ))
            }
            Some(p) => p,
        };

        Ok(application_port)
    }

    async fn up_docker(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        ctx: &mut Context,
        image: String,
    ) -> Result<String, Error> {
        let container_label = devcontainer.get_name(&self.path);

        if let Some(stat) = self
            .check_is_container_running_from_name(docker, container_label.clone())
            .await?
        {
            let id = stat.id.as_ref().unwrap();
            info!("Found container with id = '{}'", id);

            ctx.application_port = Some(self.get_application_port(Some(&stat)).await?);
            info!("Application port: {:?}", ctx.application_port.as_ref());

            // if container is not running, try to start it
            if stat.state.as_ref().unwrap() != "running" {
                docker
                    .start_container(id, None::<StartContainerOptions<String>>)
                    .await?;

                // postStartCommand
                self.run_hook(docker, devcontainer, id.clone(), CommandHook::PostStart)
                    .await?;
            }

            self.run_hook(docker, devcontainer, id.clone(), CommandHook::PostAttach)
                .await?;
            return Ok(id.clone());
        }

        ctx.application_port = Some(self.get_application_port(None).await?);
        info!("Application port: {:?}", ctx.application_port.as_ref());

        let mut config: container::Config<String> = container::Config {
            image: Some(image.clone()),
            ..Default::default()
        };

        self.container_opts_build_envs(devcontainer, ctx, &mut config)
            .await?;

        self.container_opts_build_mounts(devcontainer, &mut config)
            .await?;

        self.container_opts_build_ports(devcontainer, ctx, &mut config)
            .await?;

        self.container_opts_build_cmd(devcontainer, &mut config)
            .await?;

        let mut labels = HashMap::new();
        labels.insert("devcontainer".to_string(), "true".to_string());
        labels.insert("devcontainer_name".to_string(), container_label);

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

                    let options = Some(ListContainersOptions {
                        all: true,
                        filters,
                        ..std::default::Default::default()
                    });

                    // Check if an existing container has this name
                    if let Ok(containers) = docker.list_containers(options).await {
                        if containers.len() > 0 {
                            continue;
                        }
                    }

                    container_options = Some(CreateContainerOptions { name });

                    break;
                }
            }
        }

        let info = docker
            .create_container::<String, String>(container_options, config)
            .await?;

        let id = info.id;

        info!("Starting container");
        docker
            .start_container(id.as_str(), None::<StartContainerOptions<String>>)
            .await?;

        // postCreateCommand
        self.run_hook(docker, devcontainer, id.clone(), CommandHook::PostCreate)
            .await?;

        // postStartCommand
        self.run_hook(docker, devcontainer, id.clone(), CommandHook::PostStart)
            .await?;

        // postAttachCommand
        self.run_hook(docker, devcontainer, id.clone(), CommandHook::PostAttach)
            .await?;

        Ok(id)
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
        ctx: &mut Context,
    ) -> Result<String, Error> {
        let image = self.docker_format_image(devcontainer.image.as_ref().unwrap().to_string());

        self.docker_pull_image(docker, image.clone()).await?;

        info!("Creating container from: {}", image);
        let id = self.up_docker(&docker, devcontainer, ctx, image).await?;

        Ok(id)
    }

    async fn up_from_build(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        ctx: &mut Context,
    ) -> Result<String, Error> {
        let image = self.docker_build_image(&docker, devcontainer).await?;

        info!("Creating container from: {}", image);
        let id = self.up_docker(&docker, devcontainer, ctx, image).await?;

        Ok(id)
    }

    async fn build_docker_compose_settings_ext(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
        compose_sample_rel: PathBuf,
    ) -> Result<Option<PathBuf>, Error> {
        if let None = self.settings {
            return Ok(None);
        }

        let mut compose_sample = compose_sample_rel.clone();
        if compose_sample.is_relative() {
            compose_sample = self.get_devcontainer_folder();
            compose_sample.push(compose_sample_rel);
        }

        debug!("Building global settings compose ext");
        debug!("Compose sample: {:?}", compose_sample);
        let compose_data = fs::read_to_string(compose_sample)
            .await
            .map_err(|err| Error::Other(err.to_string()))?;

        let compose_model: SettingsComposeModel = serde_yaml::from_str(compose_data.as_str())
            .map_err(|err| Error::Other(err.to_string()))?;

        let ext_ports: Option<Vec<i32>> = match ctx.application_port.as_ref() {
            None => None,
            Some(p) => Some(vec![p.clone().into()]),
        };

        Ok(Some(
            self.settings
                .as_ref()
                .unwrap()
                .generate_compose_override(
                    devcontainer
                        .service
                        .as_ref()
                        .unwrap_or(&ctx.project_name)
                        .clone(),
                    compose_model.version,
                    Some(self.get_devcontainer_envs(devcontainer, ctx)),
                    ext_ports,
                )
                .await?,
        ))
    }

    async fn build_docker_compose_cmd(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
        extended_args: Option<Vec<String>>,
    ) -> Result<Vec<String>, Error> {
        let mut compose_args: Vec<String> = vec!["docker-compose", "-p", ctx.project_name.as_ref()]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut compose_file_sample = PathBuf::new();

        match devcontainer.docker_compose_file.as_ref().unwrap() {
            DockerComposeFile::File(file) => {
                compose_args.push("-f".to_string());
                compose_args.push(file.clone());

                compose_file_sample = PathBuf::from(&file);
            }
            DockerComposeFile::Files(files) => {
                if let Some(first) = files.first() {
                    compose_file_sample = PathBuf::from(first);
                }

                for file in files {
                    compose_args.push("-f".to_string());
                    compose_args.push(file.clone());
                }
            }
        };

        if let Some(settings_ext) = self
            .build_docker_compose_settings_ext(devcontainer, ctx, compose_file_sample)
            .await?
        {
            compose_args.push("-f".to_string());
            compose_args.push(settings_ext.into_os_string().into_string().unwrap());
        }

        if let Some(ext_args) = extended_args {
            compose_args.extend(ext_args);
        }

        Ok(compose_args)
    }

    async fn up_from_compose(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
        ctx: &mut Context,
    ) -> Result<String, Error> {
        let project_label = format!("com.docker.compose.project={}", ctx.project_name);
        let service_label = format!(
            "com.docker.compose.service={}",
            devcontainer.service.as_ref().unwrap()
        );

        let mut filters = HashMap::new();
        filters.insert(
            "label",
            vec![project_label.as_str(), service_label.as_str()],
        );

        let (existed_before, was_running_before) =
            match self.get_container_from_filters(docker, &filters).await? {
                Some(stat) => {
                    info!("Application port: {:?}", ctx.application_port.as_ref());

                    debug!("State: {}", stat.state.as_ref().unwrap());
                    (
                        true,
                        stat.state.is_some() && stat.state.as_ref().unwrap() == "running",
                    )
                }
                None => (false, false),
            };

        let mut compose_args = self
            .build_docker_compose_cmd(devcontainer, ctx, None)
            .await?;

        compose_args.push("up".to_string());
        compose_args.push("-d".to_string());

        compose_args.push(devcontainer.service.as_ref().unwrap().clone());

        if let Some(services) = devcontainer.run_services.as_ref() {
            for service in services {
                compose_args.push(service.clone());
            }
        }

        let compose_path = self.get_devcontainer_folder();

        let mut builder = &mut Command::new(compose_args[0].clone());
        builder = builder
            .args(compose_args.iter().skip(1))
            .current_dir(compose_path);

        info!("Running docker-compose");
        let compose_proc = builder
            .spawn()
            .map_err(|err| UpError::ComposeError(err.to_string()))?;

        if let Err(err) = compose_proc.await {
            return Err(Error::UpError(UpError::ComposeError(err.to_string())));
        }

        let container_stat = match self.get_container_from_filters(docker, &filters).await? {
            Some(stat) => stat,
            None => {
                return Err(Error::UpError(UpError::ContainerCreate(
                    "Could not locate container after compose up".to_string(),
                )));
            }
        };

        let container_id = container_stat.id.as_ref().unwrap();

        if !existed_before {
            // postCreateCommand
            self.run_hook(
                docker,
                devcontainer,
                container_id.clone(),
                CommandHook::PostCreate,
            )
            .await?;
        }

        if !was_running_before {
            // postStartCommand
            self.run_hook(
                docker,
                devcontainer,
                container_id.clone(),
                CommandHook::PostStart,
            )
            .await?;
        }

        // postAttachCommand
        self.run_hook(
            docker,
            devcontainer,
            container_id.clone(),
            CommandHook::PostAttach,
        )
        .await?;

        Ok(container_id.clone())
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

    fn create_context(&self, devcontainer: &DevContainer) -> Context {
        Context {
            application_port: None,
            project_name: devcontainer.get_name(&self.path),
        }
    }

    pub async fn up(&self, should_wait: bool) -> Result<(), Error> {
        let devcontainer = self.devcontainer.as_ref().ok_or(Error::NoDevContainer)?;

        let mut ctx = self.create_context(&devcontainer);

        let docker = self.create_docker_client().await?;

        info!("Starting containers");

        let container_id = match devcontainer.get_mode() {
            Mode::Image => self.up_from_image(&docker, &devcontainer, &mut ctx).await?,
            Mode::Build => self.up_from_build(&docker, &devcontainer, &mut ctx).await?,
            Mode::Compose => {
                self.up_from_compose(&docker, &devcontainer, &mut ctx)
                    .await?
            }
        };

        info!("Containers are ready: {}", container_id);

        let child = if self.settings.as_ref().unwrap().application.is_some() {
            Some(self.spawn_application(devcontainer, &ctx).await?)
        } else {
            None
        };

        info!("Should wait: {}", should_wait);
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
            return self.down(Some(docker), true).await;
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

        self.down(Some(docker), true).await
    }

    async fn down_from_image(
        &self,
        docker: &Docker,
        devcontainer: &DevContainer,
    ) -> Result<(), Error> {
        let container_label = devcontainer.get_name(&self.path);

        if let Some(stat) = self
            .check_is_container_running_from_name(docker, container_label.clone())
            .await?
        {
            let container_id = stat.id.as_ref().unwrap();

            docker
                .stop_container(container_id, None::<StopContainerOptions>)
                .await?;
        }

        Ok(())
    }

    async fn down_from_compose(
        &self,
        devcontainer: &DevContainer,
        ctx: &Context,
    ) -> Result<(), Error> {
        let compose_path = self.get_devcontainer_folder();

        let compose_args = self
            .build_docker_compose_cmd(devcontainer, ctx, Some(vec!["stop".to_string()]))
            .await?;

        let mut builder = &mut Command::new(compose_args[0].clone());
        builder = builder
            .args(compose_args.iter().skip(1))
            .current_dir(compose_path);

        info!("Running docker-compose");
        let compose_proc = builder
            .spawn()
            .map_err(|err| UpError::ComposeError(err.to_string()))?;

        if let Err(err) = compose_proc.await {
            return Err(Error::UpError(UpError::ComposeError(err.to_string())));
        }

        Ok(())
    }

    pub async fn down(&self, docker: Option<Docker>, from_up: bool) -> Result<(), Error> {
        info!("Shutting down containers");

        let devcontainer = self.devcontainer.as_ref().ok_or(Error::NoDevContainer)?;

        let mut ctx = self.create_context(&devcontainer);

        let docker = match docker {
            Some(d) => d,
            None => self.create_docker_client().await?,
        };

        let shutdown_action = devcontainer
            .shutdown_action
            .as_ref()
            .unwrap_or(&ShutdownAction::None);

        match devcontainer.get_mode() {
            Mode::Compose => {
                if from_up && shutdown_action != &ShutdownAction::StopCompose {
                    info!("Not shutting down composer. Shutdown action is not 'stopCompose'");
                    Ok(())
                } else {
                    self.down_from_compose(devcontainer, &mut ctx).await
                }
            }
            _ => {
                if from_up && shutdown_action != &ShutdownAction::StopContainer {
                    info!("Not shutting down container. Shutdown action is not 'stopContainer'");
                    Ok(())
                } else {
                    self.down_from_image(&docker, devcontainer).await
                }
            }
        }
    }
}
