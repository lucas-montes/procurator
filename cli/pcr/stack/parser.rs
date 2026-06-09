use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub cmd: serde_json::Value, // can be string or array
    #[serde(default)]
    pub src: Option<String>,
    #[serde(default)]
    pub ports: Option<Vec<u16>>,
    #[serde(rename = "dependsOn", default)]
    pub depends_on: Option<Vec<String>>, //NOTE: this should respect the order
    #[serde(rename = "oneShot", default)]
    pub one_shot: Option<bool>,
    #[serde(default)]
    // NOTE: instead of a periodic option, we could use a cron-like syntax
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
                    if port.eq(&0) {
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

pub fn parse_flake_services(repo_path: &PathBuf) -> Result<ServiceGraph, String> {
    // Run nix eval --json .#stack.services
    let output = std::process::Command::new("nix")
        .arg("eval")
        .arg("--json")
        .arg(".#stack.services")
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run nix eval: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("nix eval failed: {}", err));
    }

    let json_str = String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in nix eval output: {}", e))?;

    let services: HashMap<String, Service> = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse services JSON: {}", e))?;

    // Compute topological order
    let order = topo_sort(&services)?;

    let graph = ServiceGraph { services, order };
    graph.validate()?;

    Ok(graph)
}

fn topo_sort(services: &HashMap<String, Service>) -> Result<Vec<String>, String> {
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    // Initialize
    for name in services.keys() {
        in_degree.insert(name.clone(), 0);
        graph.insert(name.clone(), Vec::new());
    }

    // Build graph
    for (name, svc) in services {
        if let Some(deps) = &svc.depends_on {
            for dep in deps {
                if let Some(neighbors) = graph.get_mut(dep) {
                    neighbors.push(name.clone());
                }
                *in_degree.get_mut(name).unwrap() += 1;
            }
        }
    }

    // Kahn's algorithm
    let mut queue: Vec<String> = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(name, _)| name.clone())
        .collect();

    let mut order = Vec::new();

    while let Some(node) = queue.pop() {
        order.push(node.clone());

        if let Some(neighbors) = graph.get(&node) {
            for neighbor in neighbors {
                if let Some(degree) = in_degree.get_mut(neighbor) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push(neighbor.clone());
                    }
                }
            }
        }
    }

    if order.len() != services.len() {
        return Err("Cycle detected in service dependencies".to_string());
    }

    Ok(order)
}
