use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct Infrastructure {
    #[serde(rename = "continous-delivery")]
    pub continuous_delivery: ContinuousDelivery,
    pub machines: Vec<Machine>,
    pub rollback: Rollback,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ContinuousDelivery {
    pub build: bool,
    pub dst: bool,
    pub staging: bool,
    pub tests: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Machine {
    pub cpu: u32,
    pub memory: Memory,
    pub name: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Memory {
    pub amount: f64,
    pub unit: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Rollback {
    pub enabled: bool,
    pub notification: Notification,
    pub threshold: Threshold,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Notification {
    pub email: Email,
    pub enabled: bool,
    pub slack: Slack,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Email {
    pub body: String,
    pub recipients: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Slack {
    pub channel: String,
    pub message: String,
    #[serde(rename = "webhookUrl")]
    pub webhook_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Threshold {
    pub cpu: f64,
    pub latency: Latency,
    pub memory: Memory,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Latency {
    pub p50: String,
    pub p90: String,
    pub p99: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Service {
    pub production: Option<ServiceConfig>,
    pub staging: Option<Vec<ServiceConfig>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub cpu: f64,
    pub memory: Memory,
    pub packages: String,
}

pub enum State{
    Infrastructure(Infrastructure),
    Services(HashMap<String, Service>),
}


impl State {
    pub fn from_file(content: &str, section: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match section {
            "infrastructure" => Ok(Self::Infrastructure(serde_json::from_str::<Infrastructure>(content)?)),
            "services" => Ok(Self::Services(serde_json::from_str::<HashMap<String, Service>>(content)?)),
            _ => Err(format!("Unknown section: {}", section).into()),
        }
    }
}
