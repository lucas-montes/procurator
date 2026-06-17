use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    cmd: serde_json::Value,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    ports: Option<Vec<u16>>,
    #[serde(rename = "dependsOn", default)]
    depends_on: Option<Vec<String>>,
    #[serde(rename = "oneShot", default)]
    one_shot: bool,
    #[serde(default)]
    restart: Option<String>,
}

impl Service {
    pub fn cmd(&self) -> &serde_json::Value {
        &self.cmd
    }
    pub fn src(&self) -> Option<&str> {
        self.src.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct ServiceGraph {
    services: HashMap<String, Service>,
    order: Vec<String>,
}

impl ServiceGraph {
    pub fn from_services(services: HashMap<String, Service>) -> Result<Self, ParserError> {
        let order = topo_sort(&services)?;
        let graph = Self { services, order };
        graph.validate()?;
        Ok(graph)
    }

    pub fn services(&self) -> &HashMap<String, Service> {
        &self.services
    }
    pub fn order(&self) -> &[String] {
        &self.order
    }
}

/// Global log configuration, parsed from `stack.logs` in the flake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    dir: PathBuf,
    max_lines: usize,
}

impl LogConfig {
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }
    pub fn max_lines(&self) -> usize {
        self.max_lines
    }
}

/// Optional watch-mode configuration, parsed from `stack.watch` in the flake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    enable: bool,
}

impl WatchConfig {
    pub fn enabled(&self) -> bool {
        self.enable
    }
}

/// All configuration under `stack` in the flake, parsed in a single eval.
#[derive(Debug, Clone, Deserialize)]
struct StackConfig {
    services: HashMap<String, Service>,
    #[serde(default)]
    logs: Option<LogConfig>,
    #[serde(default)]
    watch: Option<WatchConfig>,
}

/// Classification of a single service's change between two graph snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceChange {
    Added,
    Removed,
    Changed,
    Unchanged,
}

/// Errors produced by flake parsing and graph validation.
#[derive(Debug)]
pub enum ParserError {
    /// `nix eval --json .#stack` failed.
    NixEval { stderr: String },
    /// JSON from nix eval could not be decoded.
    JsonDecode(serde_json::Error),
    /// I/O error running nix or reading the flake.
    Io(std::io::Error),
    /// Service dependency graph has a cycle.
    CycleDetected,
    /// A service declares port 0.
    PortInvalid { service: String, port: u16 },
    /// Two services use the same port.
    PortDuplicate { port: u16 },
    /// A service depends on a service that doesn't exist.
    DependencyUnknown { service: String, dependency: String },
    /// A non-oneShot service has no `cmd`.
    MissingCmd(String),
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParserError::NixEval { stderr } => write!(f, "nix eval failed: {}", stderr),
            ParserError::JsonDecode(e) => write!(f, "JSON parse error: {}", e),
            ParserError::Io(e) => write!(f, "I/O error: {}", e),
            ParserError::CycleDetected => write!(f, "cycle detected in dependsOn"),
            ParserError::PortInvalid { service, port } => {
                write!(f, "service {} has invalid port {}", service, port)
            }
            ParserError::PortDuplicate { port } => write!(f, "port {} is duplicated", port),
            ParserError::DependencyUnknown {
                service,
                dependency,
            } => {
                write!(
                    f,
                    "service {} depends on unknown service {}",
                    service, dependency
                )
            }
            ParserError::MissingCmd(name) => {
                write!(f, "service {} is missing a cmd", name)
            }
        }
    }
}

impl std::error::Error for ParserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParserError::JsonDecode(e) => Some(e),
            ParserError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ParserError {
    fn from(e: serde_json::Error) -> Self {
        ParserError::JsonDecode(e)
    }
}

impl From<std::io::Error> for ParserError {
    fn from(e: std::io::Error) -> Self {
        ParserError::Io(e)
    }
}

#[cfg(test)]
pub fn diff_graphs(old: &ServiceGraph, new: &ServiceGraph) -> HashMap<String, ServiceChange> {
    let mut result = HashMap::new();

    for (name, svc) in &new.services {
        let change = match old.services.get(name) {
            None => ServiceChange::Added,
            Some(old_svc) if old_svc.cmd != svc.cmd => ServiceChange::Changed,
            Some(_) => ServiceChange::Unchanged,
        };
        result.insert(name.clone(), change);
    }

    for name in old.services.keys() {
        if !new.services.contains_key(name) {
            result.insert(name.clone(), ServiceChange::Removed);
        }
    }

    result
}

impl ServiceGraph {
    pub fn validate(&self) -> Result<(), ParserError> {
        if has_cycle(&self.services) {
            return Err(ParserError::CycleDetected);
        }

        let mut seen_ports = std::collections::HashSet::new();
        for (name, svc) in &self.services {
            if let Some(ports) = &svc.ports {
                for port in ports {
                    if port.eq(&0) {
                        return Err(ParserError::PortInvalid {
                            service: name.clone(),
                            port: *port,
                        });
                    }
                    if !seen_ports.insert(*port) {
                        return Err(ParserError::PortDuplicate { port: *port });
                    }
                }
            }
        }

        for (name, svc) in &self.services {
            if let Some(deps) = &svc.depends_on {
                for dep in deps {
                    if !self.services.contains_key(dep) {
                        return Err(ParserError::DependencyUnknown {
                            service: name.clone(),
                            dependency: dep.clone(),
                        });
                    }
                }
            }
        }

        for (name, svc) in &self.services {
            if !svc.one_shot && svc.cmd.is_null() {
                return Err(ParserError::MissingCmd(name.clone()));
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

pub fn parse_stack_config(
    repo_path: &PathBuf,
) -> Result<
    (
        HashMap<String, Service>,
        Option<LogConfig>,
        Option<WatchConfig>,
    ),
    ParserError,
> {
    let output = std::process::Command::new("nix")
        .arg("eval")
        .arg("--json")
        .arg(".#stack")
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(ParserError::NixEval { stderr: err });
    }

    let raw: StackConfig = serde_json::from_slice(&output.stdout)?;
    Ok((raw.services, raw.logs, raw.watch))
}

fn topo_sort(services: &HashMap<String, Service>) -> Result<Vec<String>, ParserError> {
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    for name in services.keys() {
        in_degree.insert(name.clone(), 0);
        graph.insert(name.clone(), Vec::new());
    }

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
        return Err(ParserError::CycleDetected);
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
                    one_shot: false,
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
