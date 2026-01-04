use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfiguration {
    pub machines: HashMap<String, MachineConfig>,
    pub services: HashMap<String, ServiceConfig>,
    pub databases: HashMap<String, DatabaseConfig>,
    pub proxies: Vec<ProxyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineConfig {
    pub name: String,
    pub environment: String, // "development", "staging", "production"
    pub cpu: f64,
    pub memory: MemorySpec,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpec {
    pub amount: f64,
    pub unit: String, // "MB", "GB"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub source: ServiceSource,
    pub placement: HashMap<String, Vec<String>>, // environment -> [machine_ids]
    pub ports: Vec<PortMapping>,
    pub env_vars: HashMap<String, String>,
    pub depends_on: Vec<String>, // service/database names
    pub replicas: HashMap<String, u32>, // environment -> replica count
    pub health_check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServiceSource {
    NixPackage { package: String },
    ProjectRepo { repo_name: String, flake_output: Option<String> },
    Flake { url: String, output: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub internal: u16,
    pub external: Option<u16>,
    pub protocol: String, // "tcp", "udp"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub name: String,
    pub db_type: String, // "postgresql", "mysql", "redis", etc.
    pub package: String, // "pkgs.postgresql_16"
    pub placement: HashMap<String, Vec<String>>, // environment -> [machine_ids]
    pub storage: MemorySpec,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub name: String,
    pub package: String, // "pkgs.nginx", "pkgs.traefik"
    pub routes: Vec<RouteConfig>,
    pub tls_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub path: String,
    pub service: String,
    pub port: u16,
    pub strip_prefix: bool,
}

// DTOs for API
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigurationRequest {
    pub configuration: ProjectConfiguration,
}
