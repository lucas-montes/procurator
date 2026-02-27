use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use dockerfile_parser::{Dockerfile, Instruction, ShellOrExecExpr};

use crate::mapping::{ParseError, Parseable};

/// Container and deployment configuration files
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ContainerFile {
    // Docker ecosystem
    Dockerfile,
    DockerCompose,

    // Podman (Docker-compatible)
    Containerfile,

    // Development environment
    Skaffold, // skaffold.yaml
    Tiltfile, // Tiltfile
}

impl TryFrom<&str> for ContainerFile {
    type Error = ();

    fn try_from(filename: &str) -> Result<Self, Self::Error> {
        match filename {
            "Dockerfile" => Ok(Self::Dockerfile),
            "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
                Ok(Self::DockerCompose)
            }
            "Containerfile" => Ok(Self::Containerfile),
            "skaffold.yaml" | "skaffold.yml" => Ok(Self::Skaffold),
            "Tiltfile" => Ok(Self::Tiltfile),
            _ => Err(()),
        }
    }
}

/// Comprehensive parsed container file information
/// Contains all information that might be useful for Nix flake generation
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct ParsedContainerFile {
    /// Base image (for Dockerfile/Containerfile)
    /// For single-stage builds, this is the only image.
    /// For multi-stage builds, this is the first FROM instruction.
    /// Example: "node:20-alpine", "rust:1.75", "python:3.11-slim"
    pub base_image: Option<BaseImage>,

    /// All build stages (for multi-stage Dockerfiles)
    /// Example: [BaseImage { name: "rust", stage_name: "builder" }, BaseImage { name: "alpine" }]
    pub build_stages: Vec<BaseImage>,

    /// System packages to install (from RUN commands)
    /// Example: ["postgresql-client", "libpq-dev", "openssl", "curl"]
    pub system_packages: Vec<String>,

    /// Exposed ports
    /// Example: [3000, 8080, 9090]
    pub ports: Vec<u16>,

    /// Environment variables defined in the container
    /// Example: {"DATABASE_URL": "postgres://localhost/db", "PORT": "3000"}
    pub environment: HashMap<String, String>,

    /// Services defined (for docker-compose)
    /// Example: postgres:15, redis:7, rabbitmq:3
    pub services: Vec<ContainerService>,

    /// Command to run (CMD/ENTRYPOINT)
    /// Example: ["npm", "start"] or ["cargo", "run", "--release"]
    pub command: Option<Vec<String>>,

    /// Working directory set in the container
    /// Example: "/app", "/usr/src/app"
    pub working_dir: Option<String>,

    /// Build arguments (ARG in Dockerfile)
    /// Example: {"NODE_ENV": "production", "API_VERSION": "v2"}
    pub build_args: HashMap<String, Option<String>>,

    /// User to run as (USER in Dockerfile)
    /// Example: "node", "1000:1000"
    pub user: Option<String>,

    /// Health check configuration
    pub healthcheck: Option<HealthCheck>,

    /// Labels/metadata
    /// Example: {"version": "1.0.0", "maintainer": "team@example.com"}
    pub labels: HashMap<String, String>,
}

/// Parsed base image information
#[derive(Debug, Clone, PartialEq)]
pub struct BaseImage {
    /// Full image reference
    /// Example: "node:20-alpine"
    pub full: String,

    /// Image name without tag
    /// Example: "node"
    pub name: String,

    /// Image tag/version
    /// Example: "20-alpine", "1.75", "latest"
    pub tag: Option<String>,

    /// Distribution hint (alpine, slim, debian, etc.)
    pub distribution: Option<String>,

    /// Stage name (for multi-stage builds)
    /// Example: "builder", "runtime", "production"
    pub stage_name: Option<String>,
}

/// A service definition from docker-compose
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerService {
    /// Service name (as defined in docker-compose)
    /// Example: "api", "db", "redis", "worker"
    pub name: String,

    /// Image reference (if using pre-built image)
    /// Example: "postgres:15", "redis:7-alpine"
    pub image: Option<String>,

    /// Build context (if building from Dockerfile)
    pub build: Option<BuildConfig>,

    /// Port mappings
    /// Example: host 8080 -> container 80
    pub ports: Vec<PortMapping>,

    /// Environment variables for this service
    pub environment: HashMap<String, String>,

    /// Services this depends on
    /// Example: ["db", "redis"]
    pub depends_on: Vec<String>,

    /// Volume mounts
    pub volumes: Vec<VolumeMount>,

    /// Command override
    pub command: Option<Vec<String>>,

    /// Container healthcheck
    pub healthcheck: Option<HealthCheck>,

    /// Restart policy
    /// Example: "always", "on-failure", "unless-stopped"
    pub restart: Option<String>,
}

/// Build configuration from docker-compose
#[derive(Debug, Clone, PartialEq)]
pub struct BuildConfig {
    /// Build context path
    /// Example: ".", "./backend", "../"
    pub context: String,

    /// Dockerfile path (relative to context)
    /// Example: "Dockerfile", "docker/Dockerfile.prod"
    pub dockerfile: Option<String>,

    /// Build args
    pub args: HashMap<String, String>,

    /// Target stage (for multi-stage builds)
    pub target: Option<String>,
}

/// Port mapping between host and container
#[derive(Debug, Clone, PartialEq)]
pub struct PortMapping {
    /// Host port (None means random/published)
    pub host: Option<u16>,

    /// Container port
    pub container: u16,

    /// Protocol (tcp, udp)
    pub protocol: Option<String>,
}

/// Volume mount configuration
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMount {
    /// Volume name or host path
    /// Example: "pgdata", "./data", "/var/lib/data"
    pub source: String,

    /// Container mount path
    /// Example: "/var/lib/postgresql/data"
    pub target: String,

    /// Mount type (volume, bind, tmpfs)
    pub mount_type: Option<String>,

    /// Read-only flag
    pub read_only: bool,
}

/// Health check configuration
#[derive(Debug, Clone, PartialEq)]
pub struct HealthCheck {
    /// Test command
    /// Example: ["CMD", "curl", "-f", "http://localhost:3000/health"]
    pub test: Vec<String>,

    /// Interval between checks
    /// Example: "30s"
    pub interval: Option<String>,

    /// Timeout for each check
    /// Example: "3s"
    pub timeout: Option<String>,

    /// Number of retries before unhealthy
    /// Example: 3
    pub retries: Option<u32>,

    /// Start period before first check
    /// Example: "40s"
    pub start_period: Option<String>,
}

// Docker Compose YAML structures for deserialization
#[allow(dead_code)]
#[derive(Deserialize)]
struct DockerComposeFile {
    #[serde(default)]
    version: Option<String>,

    #[serde(default)]
    services: HashMap<String, DockerComposeService>,

    #[serde(default)]
    volumes: HashMap<String, serde_yaml_ng::Value>,

    #[serde(default)]
    networks: HashMap<String, serde_yaml_ng::Value>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct DockerComposeService {
    image: Option<String>,

    build: Option<DockerComposeBuild>,

    #[serde(default)]
    ports: Vec<DockerComposePort>,

    #[serde(default)]
    environment: DockerComposeEnvironment,

    #[serde(default)]
    depends_on: DockerComposeDependsOn,

    #[serde(default)]
    volumes: Vec<DockerComposeVolume>,

    command: Option<DockerComposeCommand>,

    healthcheck: Option<DockerComposeHealthCheck>,

    restart: Option<String>,

    working_dir: Option<String>,

    user: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeBuild {
    Simple(String),
    Complex {
        context: String,
        #[serde(default)]
        dockerfile: Option<String>,
        #[serde(default)]
        args: DockerComposeBuildArgs,
        #[serde(default)]
        target: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeBuildArgs {
    Map(HashMap<String, String>),
    List(Vec<String>),
}

impl Default for DockerComposeBuildArgs {
    fn default() -> Self {
        Self::Map(HashMap::new())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposePort {
    Short(String), // "8080:80" or "80"
    Long {
        target: u16,
        published: Option<u16>,
        protocol: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeEnvironment {
    Map(HashMap<String, String>),
    List(Vec<String>),
}

impl Default for DockerComposeEnvironment {
    fn default() -> Self {
        Self::Map(HashMap::new())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeDependsOn {
    Simple(Vec<String>),
    Complex(HashMap<String, serde_yaml_ng::Value>),
}

impl Default for DockerComposeDependsOn {
    fn default() -> Self {
        Self::Simple(Vec::new())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeVolume {
    Short(String), // "pgdata:/var/lib/postgresql/data" or "./data:/data"
    Long {
        #[serde(rename = "type")]
        mount_type: Option<String>,
        source: String,
        target: String,
        read_only: Option<bool>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeCommand {
    String(String),
    Array(Vec<String>),
}

#[derive(Deserialize)]
struct DockerComposeHealthCheck {
    test: DockerComposeHealthCheckTest,
    interval: Option<String>,
    timeout: Option<String>,
    retries: Option<u32>,
    start_period: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DockerComposeHealthCheckTest {
    String(String),
    Array(Vec<String>),
}

impl Parseable for ContainerFile {
    type Output = ParsedContainerFile;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        match self {
            Self::Dockerfile | Self::Containerfile => parse_dockerfile(path),
            Self::DockerCompose => parse_docker_compose(path),
            Self::Skaffold => parse_skaffold(path),
            Self::Tiltfile => parse_tiltfile(path),
        }
    }
}

fn parse_dockerfile(path: &Path) -> Result<ParsedContainerFile, ParseError> {
    let content = std::fs::read_to_string(path)?;
    let dockerfile = Dockerfile::parse(&content)
        .map_err(|e| ParseError::InvalidFormat(format!("Dockerfile parse error: {}", e)))?;

    let mut result = ParsedContainerFile::default();

    for instruction in dockerfile.instructions {
        match instruction {
            Instruction::From(from_inst) => {
                // Parse stage name if present (e.g., "FROM rust:1.75 as builder")
                let stage_name = from_inst.alias.as_ref().map(|s| s.content.clone());
                let mut base_image = parse_base_image(&from_inst.image.content);
                base_image.stage_name = stage_name;

                // Store first FROM as base_image for backwards compatibility
                if result.base_image.is_none() {
                    result.base_image = Some(base_image.clone());
                }

                // Track all stages for multi-stage builds
                result.build_stages.push(base_image);
            }
            Instruction::Run(run_inst) => {
                // Extract packages from RUN commands
                let command_str = match &run_inst.expr {
                    ShellOrExecExpr::Shell(s) => s.to_string(),
                    ShellOrExecExpr::Exec(arr) => arr.as_str_vec().join(" "),
                };
                result
                    .system_packages
                    .extend(extract_packages_from_run(&command_str));
            }
            Instruction::Env(env_inst) => {
                for var in env_inst.vars {
                    result
                        .environment
                        .insert(var.key.content.clone(), var.value.to_string());
                }
            }
            Instruction::Cmd(cmd_inst) => {
                result.command = Some(match &cmd_inst.expr {
                    ShellOrExecExpr::Shell(s) => vec![s.to_string()],
                    ShellOrExecExpr::Exec(arr) => {
                        arr.as_str_vec().iter().map(|s| s.to_string()).collect()
                    }
                });
            }
            Instruction::Entrypoint(entrypoint_inst) => {
                result.command = Some(match &entrypoint_inst.expr {
                    ShellOrExecExpr::Shell(s) => vec![s.to_string()],
                    ShellOrExecExpr::Exec(arr) => {
                        arr.as_str_vec().iter().map(|s| s.to_string()).collect()
                    }
                });
            }
            Instruction::Arg(arg_inst) => {
                result.build_args.insert(
                    arg_inst.name.content.clone(),
                    arg_inst.value.map(|v| v.content),
                );
            }
            Instruction::Label(label_inst) => {
                for label in label_inst.labels {
                    result
                        .labels
                        .insert(label.name.content.clone(), label.value.content.clone());
                }
            }
            Instruction::Misc(misc) => {
                // Handle instructions not directly exposed by the parser
                let instruction_name = &misc.instruction.content;
                match instruction_name.as_str() {
                    "EXPOSE" => {
                        // Parse EXPOSE ports
                        let args_str = misc.arguments.to_string();
                        for port_str in args_str.split_whitespace() {
                            if let Some(port_num) = port_str.split('/').next() {
                                if let Ok(port) = port_num.parse::<u16>() {
                                    result.ports.push(port);
                                }
                            }
                        }
                    }
                    "WORKDIR" => {
                        result.working_dir = Some(misc.arguments.to_string().trim().to_string());
                    }
                    "USER" => {
                        result.user = Some(misc.arguments.to_string().trim().to_string());
                    }
                    "HEALTHCHECK" => {
                        // HEALTHCHECK parsing is complex, skip for now
                        // The test fixture has it but we'll mark as unsupported
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(result)
}

fn parse_base_image(image: &str) -> BaseImage {
    let (name_part, tag) = if let Some(colon_pos) = image.rfind(':') {
        let name = &image[..colon_pos];
        let tag = &image[colon_pos + 1..];
        (name, Some(tag.to_string()))
    } else {
        (image, None)
    };

    // Extract distribution hint from tag
    let distribution = tag.as_ref().and_then(|t| {
        if t.contains("alpine") {
            Some("alpine".to_string())
        } else if t.contains("slim") {
            Some("slim".to_string())
        } else if t.contains("debian") {
            Some("debian".to_string())
        } else if t.contains("ubuntu") {
            Some("ubuntu".to_string())
        } else {
            None
        }
    });

    BaseImage {
        full: image.to_string(),
        name: name_part.to_string(),
        tag,
        distribution,
        stage_name: None,
    }
}

fn extract_packages_from_run(command: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let lower = command.to_lowercase();

    // apt-get/apt install
    if lower.contains("apt-get install") || lower.contains("apt install") {
        if let Some(start) = lower.find("install") {
            let after_install = &command[start + 7..];
            for word in after_install.split_whitespace() {
                if !word.starts_with('-')
                    && !matches!(
                        word,
                        "install" | "apt-get" | "apt" | "&&" | "||" | "update" | "-y"
                    )
                {
                    packages.push(word.to_string());
                }
            }
        }
    }

    // apk add (Alpine)
    if lower.contains("apk add") {
        if let Some(start) = lower.find("add") {
            let after_add = &command[start + 3..];
            for word in after_add.split_whitespace() {
                if !word.starts_with('-')
                    && !matches!(word, "add" | "apk" | "&&" | "||" | "--no-cache")
                {
                    packages.push(word.to_string());
                }
            }
        }
    }

    // yum/dnf install
    if lower.contains("yum install") || lower.contains("dnf install") {
        if let Some(start) = lower.find("install") {
            let after_install = &command[start + 7..];
            for word in after_install.split_whitespace() {
                if !word.starts_with('-')
                    && !matches!(word, "install" | "yum" | "dnf" | "&&" | "||" | "-y")
                {
                    packages.push(word.to_string());
                }
            }
        }
    }

    packages
}

fn parse_docker_compose(path: &Path) -> Result<ParsedContainerFile, ParseError> {
    let content = std::fs::read_to_string(path)?;
    let compose: DockerComposeFile = serde_yaml_ng::from_str(&content)?;

    let mut result = ParsedContainerFile::default();

    for (name, service) in compose.services {
        let environment = match service.environment {
            DockerComposeEnvironment::Map(map) => map,
            DockerComposeEnvironment::List(list) => {
                let mut map = HashMap::new();
                for item in list {
                    if let Some((key, value)) = item.split_once('=') {
                        map.insert(key.to_string(), value.to_string());
                    }
                }
                map
            }
        };

        let ports: Vec<PortMapping> = service
            .ports
            .into_iter()
            .filter_map(|port| parse_port_mapping(port))
            .collect();

        let depends_on = match service.depends_on {
            DockerComposeDependsOn::Simple(list) => list,
            DockerComposeDependsOn::Complex(map) => map.keys().cloned().collect(),
        };

        let volumes: Vec<VolumeMount> = service
            .volumes
            .into_iter()
            .filter_map(|vol| parse_volume_mount(vol))
            .collect();

        let build = service.build.map(|b| match b {
            DockerComposeBuild::Simple(context) => BuildConfig {
                context,
                dockerfile: None,
                args: HashMap::new(),
                target: None,
            },
            DockerComposeBuild::Complex {
                context,
                dockerfile,
                args,
                target,
            } => {
                let args_map = match args {
                    DockerComposeBuildArgs::Map(map) => map,
                    DockerComposeBuildArgs::List(list) => {
                        let mut map = HashMap::new();
                        for item in list {
                            if let Some((key, value)) = item.split_once('=') {
                                map.insert(key.to_string(), value.to_string());
                            }
                        }
                        map
                    }
                };

                BuildConfig {
                    context,
                    dockerfile,
                    args: args_map,
                    target,
                }
            }
        });

        let command = service.command.map(|cmd| match cmd {
            DockerComposeCommand::String(s) => s.split_whitespace().map(String::from).collect(),
            DockerComposeCommand::Array(arr) => arr,
        });

        let healthcheck = service.healthcheck.map(|hc| {
            let test = match hc.test {
                DockerComposeHealthCheckTest::String(s) => {
                    s.split_whitespace().map(String::from).collect()
                }
                DockerComposeHealthCheckTest::Array(arr) => arr,
            };

            HealthCheck {
                test,
                interval: hc.interval,
                timeout: hc.timeout,
                retries: hc.retries,
                start_period: hc.start_period,
            }
        });

        result.services.push(ContainerService {
            name,
            image: service.image,
            build,
            ports,
            environment,
            depends_on,
            volumes,
            command,
            healthcheck,
            restart: service.restart,
        });
    }

    Ok(result)
}

fn parse_port_mapping(port: DockerComposePort) -> Option<PortMapping> {
    match port {
        DockerComposePort::Short(s) => {
            // Parse "8080:80" or "80"
            if let Some((host, container)) = s.split_once(':') {
                let host_port = host.parse::<u16>().ok();
                let container_port = container.parse::<u16>().ok()?;
                Some(PortMapping {
                    host: host_port,
                    container: container_port,
                    protocol: None,
                })
            } else {
                let port = s.parse::<u16>().ok()?;
                Some(PortMapping {
                    host: None,
                    container: port,
                    protocol: None,
                })
            }
        }
        DockerComposePort::Long {
            target,
            published,
            protocol,
        } => Some(PortMapping {
            host: published,
            container: target,
            protocol,
        }),
    }
}

fn parse_volume_mount(volume: DockerComposeVolume) -> Option<VolumeMount> {
    match volume {
        DockerComposeVolume::Short(s) => {
            // Parse "pgdata:/var/lib/postgresql/data" or "./data:/data:ro"
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() >= 2 {
                let read_only = parts.get(2).map(|&s| s == "ro").unwrap_or(false);
                Some(VolumeMount {
                    source: parts[0].to_string(),
                    target: parts[1].to_string(),
                    mount_type: None,
                    read_only,
                })
            } else {
                None
            }
        }
        DockerComposeVolume::Long {
            mount_type,
            source,
            target,
            read_only,
        } => Some(VolumeMount {
            source,
            target,
            mount_type,
            read_only: read_only.unwrap_or(false),
        }),
    }
}

fn parse_skaffold(_path: &Path) -> Result<ParsedContainerFile, ParseError> {
    // TODO: Implement skaffold.yaml parsing if needed
    Ok(ParsedContainerFile::default())
}

fn parse_tiltfile(_path: &Path) -> Result<ParsedContainerFile, ParseError> {
    // TODO: Implement Tiltfile parsing if needed
    Ok(ParsedContainerFile::default())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_container_file_try_from() {
        assert_eq!(
            ContainerFile::try_from("Dockerfile"),
            Ok(ContainerFile::Dockerfile)
        );
        assert_eq!(
            ContainerFile::try_from("docker-compose.yml"),
            Ok(ContainerFile::DockerCompose)
        );
        assert_eq!(
            ContainerFile::try_from("Containerfile"),
            Ok(ContainerFile::Containerfile)
        );
        assert!(ContainerFile::try_from("random.txt").is_err());
    }

    #[test]
    fn test_parse_base_image() {
        let img = parse_base_image("node:20-alpine");
        assert_eq!(img.name, "node");
        assert_eq!(img.tag, Some("20-alpine".to_string()));
        assert_eq!(img.distribution, Some("alpine".to_string()));
        assert_eq!(img.stage_name, None);

        let img = parse_base_image("rust:1.75");
        assert_eq!(img.name, "rust");
        assert_eq!(img.tag, Some("1.75".to_string()));
        assert_eq!(img.distribution, None);
        assert_eq!(img.stage_name, None);

        let img = parse_base_image("python:3.11-slim");
        assert_eq!(img.name, "python");
        assert_eq!(img.tag, Some("3.11-slim".to_string()));
        assert_eq!(img.distribution, Some("slim".to_string()));
        assert_eq!(img.stage_name, None);
    }

    #[test]
    fn test_extract_packages_from_run() {
        let apt = "apt-get update && apt-get install -y postgresql-client libpq-dev";
        let packages = extract_packages_from_run(apt);
        assert!(packages.contains(&"postgresql-client".to_string()));
        assert!(packages.contains(&"libpq-dev".to_string()));

        let apk = "apk add --no-cache redis curl openssl";
        let packages = extract_packages_from_run(apk);
        assert!(packages.contains(&"redis".to_string()));
        assert!(packages.contains(&"curl".to_string()));
        assert!(packages.contains(&"openssl".to_string()));
    }

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("containers")
    }

    #[test]
    fn test_parse_dockerfile() {
        let path = fixtures_path().join("Dockerfile");
        let container_file = ContainerFile::Dockerfile;
        let result = container_file
            .parse(&path)
            .expect("Failed to parse Dockerfile");

        // Check base image (first stage)
        assert!(result.base_image.is_some());
        let base = result.base_image.as_ref().unwrap();
        assert_eq!(base.name, "rust");
        assert_eq!(base.tag, Some("1.75-alpine".to_string()));
        assert_eq!(base.distribution, Some("alpine".to_string()));
        assert_eq!(base.stage_name, Some("builder".to_string()));

        // Check all build stages (multi-stage build)
        assert_eq!(result.build_stages.len(), 2);
        assert_eq!(result.build_stages[0].name, "rust");
        assert_eq!(
            result.build_stages[0].stage_name,
            Some("builder".to_string())
        );
        assert_eq!(result.build_stages[1].name, "alpine");
        assert_eq!(result.build_stages[1].tag, Some("3.18".to_string()));
        assert_eq!(result.build_stages[1].stage_name, None);

        // Check system packages
        assert!(!result.system_packages.is_empty());
        assert!(
            result.system_packages.contains(&"openssl".to_string())
                || result.system_packages.contains(&"openssl-dev".to_string())
        );
        assert!(result.system_packages.contains(&"curl".to_string()));

        // Check exposed ports
        assert_eq!(result.ports.len(), 2);
        assert!(result.ports.contains(&8080));
        assert!(result.ports.contains(&9090));

        // Check environment variables
        assert!(result.environment.contains_key("RUST_LOG"));
        assert_eq!(
            result.environment.get("RUST_LOG"),
            Some(&"info".to_string())
        );
        assert!(result.environment.contains_key("APP_PORT"));
        assert!(result.environment.contains_key("DATABASE_URL"));

        // Check working directory
        assert_eq!(result.working_dir, Some("/app".to_string()));

        // Check user
        assert_eq!(result.user, Some("app".to_string()));

        // Check labels
        assert!(result.labels.contains_key("version"));
        assert_eq!(result.labels.get("version"), Some(&"1.0.0".to_string()));
        assert!(result.labels.contains_key("maintainer"));
        assert!(result.labels.contains_key("description"));

        // Check command
        assert!(result.command.is_some());
        let cmd = result.command.as_ref().unwrap();
        assert_eq!(cmd, &vec!["/app/myapp".to_string()]);

        // Note: HEALTHCHECK parsing is not fully supported by the dockerfile-parser library
    }

    #[test]
    fn test_parse_docker_compose() {
        let path = fixtures_path().join("docker-compose.yml");
        let container_file = ContainerFile::DockerCompose;
        let result = container_file
            .parse(&path)
            .expect("Failed to parse docker-compose.yml");

        // Check services
        assert_eq!(result.services.len(), 5);

        // Check API service
        let api = result
            .services
            .iter()
            .find(|s| s.name == "api")
            .expect("API service not found");
        assert!(api.build.is_some());
        let build = api.build.as_ref().unwrap();
        assert_eq!(build.context, ".");
        assert_eq!(build.dockerfile, Some("Dockerfile".to_string()));
        assert_eq!(build.target, Some("production".to_string()));
        assert_eq!(build.args.get("NODE_ENV"), Some(&"production".to_string()));
        assert_eq!(build.args.get("API_VERSION"), Some(&"v2".to_string()));

        assert_eq!(api.ports.len(), 2);
        assert!(
            api.ports
                .iter()
                .any(|p| p.container == 8080 && p.host == Some(8080))
        );
        assert!(
            api.ports
                .iter()
                .any(|p| p.container == 9090 && p.host == Some(9090))
        );

        assert!(api.environment.contains_key("DATABASE_URL"));
        assert!(api.environment.contains_key("REDIS_URL"));
        assert_eq!(api.environment.get("LOG_LEVEL"), Some(&"info".to_string()));

        assert_eq!(api.depends_on.len(), 2);
        assert!(api.depends_on.contains(&"db".to_string()));
        assert!(api.depends_on.contains(&"cache".to_string()));

        assert_eq!(api.restart, Some("unless-stopped".to_string()));

        assert!(api.healthcheck.is_some());
        let healthcheck = api.healthcheck.as_ref().unwrap();
        assert_eq!(healthcheck.interval, Some("30s".to_string()));
        assert_eq!(healthcheck.timeout, Some("5s".to_string()));
        assert_eq!(healthcheck.retries, Some(3));

        // Check worker service
        let worker = result
            .services
            .iter()
            .find(|s| s.name == "worker")
            .expect("Worker service not found");
        assert_eq!(worker.image, Some("myapp:latest".to_string()));
        assert_eq!(worker.restart, Some("always".to_string()));

        // Check database service
        let db = result
            .services
            .iter()
            .find(|s| s.name == "db")
            .expect("DB service not found");
        assert_eq!(db.image, Some("postgres:15-alpine".to_string()));
        assert_eq!(
            db.environment.get("POSTGRES_PASSWORD"),
            Some(&"password".to_string())
        );
        assert_eq!(
            db.environment.get("POSTGRES_DB"),
            Some(&"myapp".to_string())
        );
        assert!(!db.volumes.is_empty());

        // Check cache service (redis)
        let cache = result
            .services
            .iter()
            .find(|s| s.name == "cache")
            .expect("Cache service not found");
        assert_eq!(cache.image, Some("redis:7-alpine".to_string()));

        // Check port with Long format
        assert!(!cache.ports.is_empty());
        let redis_port = &cache.ports[0];
        assert_eq!(redis_port.container, 6379);
        assert_eq!(redis_port.host, Some(6379));
        assert_eq!(redis_port.protocol, Some("tcp".to_string()));

        // Check nginx service
        let nginx = result
            .services
            .iter()
            .find(|s| s.name == "nginx")
            .expect("Nginx service not found");
        assert_eq!(nginx.image, Some("nginx:alpine".to_string()));
        assert_eq!(nginx.depends_on, vec!["api".to_string()]);

        // Check volume with Long format
        assert!(!nginx.volumes.is_empty());
        let bind_volume = &nginx.volumes[0];
        assert_eq!(bind_volume.source, "./nginx.conf");
        assert_eq!(bind_volume.target, "/etc/nginx/nginx.conf");
        assert_eq!(bind_volume.mount_type, Some("bind".to_string()));
        assert!(bind_volume.read_only);
    }

    #[test]
    fn test_parse_port_mapping_short() {
        let short_port = DockerComposePort::Short("8080:80".to_string());
        let mapping = parse_port_mapping(short_port).unwrap();
        assert_eq!(mapping.host, Some(8080));
        assert_eq!(mapping.container, 80);

        let container_only = DockerComposePort::Short("3000".to_string());
        let mapping = parse_port_mapping(container_only).unwrap();
        assert_eq!(mapping.host, None);
        assert_eq!(mapping.container, 3000);
    }

    #[test]
    fn test_parse_port_mapping_long() {
        let long_port = DockerComposePort::Long {
            target: 80,
            published: Some(8080),
            protocol: Some("tcp".to_string()),
        };
        let mapping = parse_port_mapping(long_port).unwrap();
        assert_eq!(mapping.host, Some(8080));
        assert_eq!(mapping.container, 80);
        assert_eq!(mapping.protocol, Some("tcp".to_string()));
    }

    #[test]
    fn test_parse_volume_mount_short() {
        let named_volume = DockerComposeVolume::Short("data:/app/data".to_string());
        let mount = parse_volume_mount(named_volume).unwrap();
        assert_eq!(mount.source, "data");
        assert_eq!(mount.target, "/app/data");
        assert!(!mount.read_only);

        let bind_mount = DockerComposeVolume::Short("./config:/etc/config:ro".to_string());
        let mount = parse_volume_mount(bind_mount).unwrap();
        assert_eq!(mount.source, "./config");
        assert_eq!(mount.target, "/etc/config");
        assert!(mount.read_only);
    }

    #[test]
    fn test_parse_volume_mount_long() {
        let long_volume = DockerComposeVolume::Long {
            mount_type: Some("bind".to_string()),
            source: "./data".to_string(),
            target: "/app/data".to_string(),
            read_only: Some(true),
        };
        let mount = parse_volume_mount(long_volume).unwrap();
        assert_eq!(mount.source, "./data");
        assert_eq!(mount.target, "/app/data");
        assert_eq!(mount.mount_type, Some("bind".to_string()));
        assert!(mount.read_only);
    }
}
