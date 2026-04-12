use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfiguration {
    pub services: HashMap<String, ServiceConfig>,
    pub dependencies: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub connection_type: ConnectionType,
    pub config: ConnectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Http,
    Database,
    Cache,
    Queue,
    Grpc,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionConfig {
    pub port: Option<u16>,
    pub protocol: Option<String>,
    pub endpoint: Option<String>,
    pub env_var_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpec {
    pub amount: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub source: ServiceSource,
    pub service_type: ServiceType,
    pub environments: HashMap<String, EnvironmentConfig>,
    pub ports: Vec<PortMapping>,
    pub env_vars: HashMap<String, String>,
    pub health_check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Application,
    Database,
    Cache,
    Proxy,
    Queue,
    Storage,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    pub enabled: bool,
    pub resources: ResourceRequirements,
    pub replicas: u32,
    pub env_vars: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServiceSource {
    ProjectRepo {
        repo_name: String,
        flake_output: Option<String>,
    },
    NixPackage {
        package: String,
    },
    Flake {
        url: String,
        output: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub cpu: f64,
    pub memory: MemorySpec,
    pub storage: Option<MemorySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub internal: u16,
    pub external: Option<u16>,
    pub protocol: String,
}
