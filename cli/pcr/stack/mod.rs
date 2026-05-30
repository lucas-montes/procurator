mod cli;
mod commands;
mod parser;

pub use cli::StackArgs;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub cmd: serde_json::Value, // can be string or array
    #[serde(default)]
    pub src: Option<String>,
    #[serde(default)]
    pub ports: Option<Vec<u16>>,
    #[serde(rename = "dependsOn", default)]
    pub depends_on: Option<Vec<String>>,
    #[serde(rename = "oneShot", default)]
    pub one_shot: Option<bool>,
    #[serde(default)]
    pub restart: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServiceGraph {
    pub services: HashMap<String, Service>,
    pub order: Vec<String>, // topologically sorted service names
}

impl ServiceGraph {
    pub fn validate(&self) -> Result<(), String> {
        // Check for cycles in dependsOn
        if has_cycle(&self.services) {
            return Err("Cycle detected in dependsOn".to_string());
        }

        // Check for unique ports
        let mut seen_ports = std::collections::HashSet::new();
        for (name, svc) in &self.services {
            if let Some(ports) = &svc.ports {
                for port in ports {
                    if *port == 0 || *port > 65535 {
                        return Err(format!("Service {} has invalid port {}", name, port));
                    }
                    if !seen_ports.insert(*port) {
                        return Err(format!("Port {} is duplicated", port));
                    }
                }
            }
        }

        // Check that all dependsOn references exist
        for (name, svc) in &self.services {
            if let Some(deps) = &svc.depends_on {
                for dep in deps {
                    if !self.services.contains_key(dep) {
                        return Err(format!(
                            "Service {} depends on unknown service {}",
                            name, dep
                        ));
                    }
                }
            }
        }

        // Check cmd is present for non-oneShot services
        for (name, svc) in &self.services {
            if !svc.one_shot.unwrap_or(false) && svc.cmd.is_null() {
                return Err(format!("Service {} has no cmd", name));
            }
        }

        Ok(())
    }
}

fn has_cycle(services: &HashMap<String, Service>) -> bool {
    use std::collections::HashSet;

    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    fn dfs(
        node: &str,
        services: &HashMap<String, Service>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());

        if let Some(svc) = services.get(node) {
            if let Some(deps) = &svc.depends_on {
                for dep in deps {
                    if !visited.contains(dep) {
                        if dfs(dep, services, visited, rec_stack) {
                            return true;
                        }
                    } else if rec_stack.contains(dep) {
                        return true;
                    }
                }
            }
        }

        rec_stack.remove(node);
        false
    }

    for name in services.keys() {
        if !visited.contains(name) {
            if dfs(name, services, &mut visited, &mut rec_stack) {
                return true;
            }
        }
    }

    false
}
