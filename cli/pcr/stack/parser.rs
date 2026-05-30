use super::{Service, ServiceGraph};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn parse_and_run(repo_path: &PathBuf) -> Result<(), String> {
    let graph = parse_flake_services(repo_path)?;
    run_stack(graph, repo_path)?;
    Ok(())
}

fn parse_flake_services(repo_path: &PathBuf) -> Result<ServiceGraph, String> {
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

fn run_stack(graph: ServiceGraph, repo_path: &PathBuf) -> Result<(), String> {
    let rt =
        tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;
    rt.block_on(async { run_stack_async(graph, repo_path).await })
}

async fn run_stack_async(graph: ServiceGraph, repo_path: &PathBuf) -> Result<(), String> {
    use std::process::Stdio;
    use tokio::io::AsyncBufReadExt;

    let mut children = Vec::new();

    println!("Starting services in order: {:?}", graph.order);

    for svc_name in &graph.order {
        let svc = graph.services.get(svc_name).unwrap();

        println!("[{}] Starting...", svc_name);

        // Prepare command
        let (prog, args) = match &svc.cmd {
            serde_json::Value::String(s) => {
                // Shell command
                ("sh".to_string(), vec!["-c".to_string(), s.clone()])
            }
            serde_json::Value::Array(arr) => {
                // Exec form
                let mut iter = arr.iter();
                let prog = iter
                    .next()
                    .and_then(|v| v.as_str())
                    .unwrap_or("sh")
                    .to_string();
                let args = iter
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                (prog, args)
            }
            _ => return Err(format!("Invalid cmd for service {}", svc_name)),
        };

        let mut work_dir = repo_path.clone();
        if let Some(src) = &svc.src {
            work_dir.push(src);
        }

        let mut cmd = tokio::process::Command::new(&prog);
        for arg in args {
            cmd.arg(arg);
        }

        cmd.current_dir(&work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn {}: {}", svc_name, e))?;

        let name = svc_name.clone();
        if let Some(stdout) = child.stdout.take() {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            let name_clone = name.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = reader.next_line().await {
                    println!("[{}] {}", name_clone, line);
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            let name_clone = name.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = reader.next_line().await {
                    eprintln!("[{}] ERR {}", name_clone, line);
                }
            });
        }

        children.push((name.clone(), child));
    }

    println!("All services started. Press Ctrl-C to stop...");

    // Wait indefinitely for children
    if !children.is_empty() {
        let _ = children[0].1.wait().await;
    }

    println!("\nShutting down services...");

    // Stop children in reverse order
    for (name, mut child) in children.into_iter().rev() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        println!("[{}] stopped", name);
    }

    Ok(())
}
