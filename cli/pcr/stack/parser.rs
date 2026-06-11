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
    /// Build a graph from a raw services map (sorts and validates).
    pub fn from_services(services: HashMap<String, Service>) -> Result<Self, String> {
        let order = topo_sort(&services)?;
        let graph = Self { services, order };
        graph.validate()?;
        Ok(graph)
    }
}

/// Global log configuration, parsed from `stack.logs` in the flake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Directory for log files (relative to --path, or absolute).
    pub dir: PathBuf,
    /// Max total lines across all services before rotating to a new file.
    pub max_lines: usize,
}

/// Optional watch-mode configuration, parsed from `stack.watch` in the flake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    /// Enable hot-reload watch mode.
    pub enable: bool,
    /// Also watch `flake.nix` for service-level changes.
    #[serde(default)]
    pub watch_flake: bool,
}

/// All configuration under `stack` in the flake, parsed in a single eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackConfig {
    pub services: HashMap<String, Service>,
    #[serde(default)]
    pub logs: Option<LogConfig>,
    #[serde(default)]
    pub watch: Option<WatchConfig>,
}

/// Classification of a single service's change between two graph snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceChange {
    /// Present in new graph but not in old.
    Added,
    /// Present in old graph but not in new.
    Removed,
    /// Present in both but `cmd` differs.
    Changed,
    /// Present in both with identical `cmd`.
    Unchanged,
}

/// Diff two [`ServiceGraph`] snapshots and classify every service.
///
/// The comparison is based on the `cmd` field serialised as JSON.
pub fn diff_graphs(old: &ServiceGraph, new: &ServiceGraph) -> HashMap<String, ServiceChange> {
    let mut result = HashMap::new();

    // Services in new graph
    for (name, svc) in &new.services {
        let change = match old.services.get(name) {
            None => ServiceChange::Added,
            Some(old_svc) if old_svc.cmd != svc.cmd => ServiceChange::Changed,
            Some(_) => ServiceChange::Unchanged,
        };
        result.insert(name.clone(), change);
    }

    // Services only in old graph (removed)
    for name in old.services.keys() {
        if !new.services.contains_key(name) {
            result.insert(name.clone(), ServiceChange::Removed);
        }
    }

    result
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

/// Parse all config from `.#stack` in a single `nix eval --json` call.
///
/// Returns the raw services map and optional log/watch config.
pub fn parse_stack_config(
    repo_path: &PathBuf,
) -> Result<
    (
        HashMap<String, Service>,
        Option<LogConfig>,
        Option<WatchConfig>,
    ),
    String,
> {
    let output = std::process::Command::new("nix")
        .arg("eval")
        .arg("--json")
        .arg(".#stack")
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run nix eval: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("nix eval failed: {}", err));
    }

    let raw: StackConfig = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse nix eval output: {}", e))?;

    Ok((raw.services, raw.logs, raw.watch))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph(pairs: &[(&str, &str)]) -> ServiceGraph {
        let mut services = HashMap::new();
        for (name, cmd_str) in pairs {
            services.insert(
                name.to_string(),
                Service {
                    cmd: serde_json::json!(cmd_str),
                    src: None,
                    ports: None,
                    depends_on: None,
                    one_shot: None,
                    restart: None,
                },
            );
        }
        let order: Vec<String> = pairs.iter().map(|(n, _)| n.to_string()).collect();
        ServiceGraph { services, order }
    }

    #[test]
    fn test_diff_added() {
        let old = make_graph(&[("a", "echo 1")]);
        let new = make_graph(&[("a", "echo 1"), ("b", "echo 2")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff["a"], ServiceChange::Unchanged);
        assert_eq!(diff["b"], ServiceChange::Added);
    }

    #[test]
    fn test_diff_removed() {
        let old = make_graph(&[("a", "echo 1"), ("b", "echo 2")]);
        let new = make_graph(&[("a", "echo 1")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff["a"], ServiceChange::Unchanged);
        assert_eq!(diff["b"], ServiceChange::Removed);
    }

    #[test]
    fn test_diff_changed_cmd() {
        let old = make_graph(&[("a", "echo 1")]);
        let new = make_graph(&[("a", "echo 2")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff["a"], ServiceChange::Changed);
    }

    #[test]
    fn test_diff_unchanged() {
        let old = make_graph(&[("a", "echo 1"), ("b", "echo 2")]);
        let new = make_graph(&[("a", "echo 1"), ("b", "echo 2")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff["a"], ServiceChange::Unchanged);
        assert_eq!(diff["b"], ServiceChange::Unchanged);
    }

    #[test]
    fn test_diff_empty_vs_populated() {
        let old = make_graph(&[]);
        let new = make_graph(&[("a", "echo 1")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff["a"], ServiceChange::Added);
    }

    #[test]
    fn test_diff_populated_vs_empty() {
        let old = make_graph(&[("a", "echo 1")]);
        let new = make_graph(&[]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff["a"], ServiceChange::Removed);
    }

    #[test]
    fn test_diff_add_remove_change() {
        let old = make_graph(&[("a", "echo 1"), ("b", "echo 2")]);
        let new = make_graph(&[("b", "echo 99"), ("c", "echo 3")]);
        let diff = diff_graphs(&old, &new);
        assert_eq!(diff.len(), 3);
        assert_eq!(diff["a"], ServiceChange::Removed);
        assert_eq!(diff["b"], ServiceChange::Changed);
        assert_eq!(diff["c"], ServiceChange::Added);
    }
}
