use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfiguration {
    pub services: HashMap<String, ServiceConfig>,
    pub dependencies: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: String, // service name
    pub to: String,   // service name
    pub connection_type: ConnectionType,
    pub config: ConnectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Http,      // HTTP/REST API calls
    Database,  // Database connection
    Cache,     // Cache/Redis connection
    Queue,     // Message queue
    Grpc,      // gRPC connection
    Custom,    // Custom protocol
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionConfig {
    pub port: Option<u16>,
    pub protocol: Option<String>,
    pub endpoint: Option<String>,
    pub env_var_name: Option<String>, // e.g., "DATABASE_URL"
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
    pub service_type: ServiceType,
    pub environments: HashMap<String, EnvironmentConfig>, // "development", "staging", "production"
    pub ports: Vec<PortMapping>,
    pub env_vars: HashMap<String, String>, // Common env vars across all environments
    pub health_check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Application, // User's application service (from project repo)
    Database,    // Database service (postgres, mysql, etc.)
    Cache,       // Cache service (redis, memcached)
    Proxy,       // Reverse proxy (nginx, traefik)
    Queue,       // Message queue (rabbitmq, kafka)
    Storage,     // Storage service (minio, s3)
    Other,       // Other infrastructure
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    pub enabled: bool,
    pub resources: ResourceRequirements,
    pub replicas: u32,
    pub env_vars: HashMap<String, String>, // Environment-specific overrides
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServiceSource {
    ProjectRepo {
        repo_name: String,
        flake_output: Option<String>
    },
    NixPackage {
        package: String // e.g., "pkgs.postgresql_16", "pkgs.nginx"
    },
    Flake {
        url: String,
        output: String
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub cpu: f64,           // Number of CPU cores
    pub memory: MemorySpec, // RAM allocation
    pub storage: Option<MemorySpec>, // Disk storage (for databases, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub internal: u16,
    pub external: Option<u16>,
    pub protocol: String, // "tcp", "udp"
}



// DTOs for API
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigurationRequest {
    pub configuration: ProjectConfiguration,
}
