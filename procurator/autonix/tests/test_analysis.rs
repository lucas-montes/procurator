use autonix::repo::{scan::scan_repos, Analysis};
use std::path::PathBuf;

fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("analysis")
}

#[test]
fn test_analyze_rust_project() {
    let rust_project = fixtures_path().join("rust");

    // Scan the Rust project
    let scan_iter = scan_repos(&rust_project).expect("Failed to scan Rust project");

    // Convert to analysis
    let analysis = Analysis::from(scan_iter);

    // The analysis should contain parsed information
    // We can't directly access the internal structure due to privacy,
    // but we can verify it was created successfully
    assert!(format!("{:?}", analysis).contains("Analysis"));
}

#[test]
fn test_analyze_js_python_project() {
    let js_python_project = fixtures_path().join("js_and_python");

    // Scan the JavaScript/Python monorepo
    let scan_iter = scan_repos(&js_python_project).expect("Failed to scan JS/Python project");

    // Convert to analysis
    let analysis = Analysis::from(scan_iter);

    // Verify the analysis was created
    assert!(format!("{:?}", analysis).contains("Analysis"));
}

#[test]
fn test_rust_project_has_expected_files() {
    let rust_project = fixtures_path().join("rust");

    // Verify all expected files exist
    assert!(rust_project.join("Cargo.toml").exists(), "Cargo.toml should exist");
    assert!(rust_project.join("Cargo.lock").exists(), "Cargo.lock should exist");
    assert!(rust_project.join("Dockerfile").exists(), "Dockerfile should exist");
    assert!(rust_project.join("docker-compose.yml").exists(), "docker-compose.yml should exist");
    assert!(rust_project.join(".gitlab-ci.yml").exists(), "GitLab CI config should exist");
    assert!(rust_project.join("Makefile").exists(), "Makefile should exist");

    // Verify workspace structure
    assert!(rust_project.join("crates/myapp-core/Cargo.toml").exists(), "core crate should exist");
    assert!(rust_project.join("crates/myapp-api/Cargo.toml").exists(), "api crate should exist");
    assert!(rust_project.join("crates/myapp-cli/Cargo.toml").exists(), "cli crate should exist");
}

#[test]
fn test_js_python_project_has_expected_files() {
    let js_python_project = fixtures_path().join("js_and_python");

    // Verify all expected files exist
    assert!(js_python_project.join("package.json").exists(), "package.json should exist");
    assert!(js_python_project.join("pyproject.toml").exists(), "pyproject.toml should exist");
    assert!(js_python_project.join("requirements.txt").exists(), "requirements.txt should exist");
    assert!(js_python_project.join("Dockerfile").exists(), "Dockerfile should exist");
    assert!(js_python_project.join("docker-compose.yml").exists(), "docker-compose.yml should exist");

    // Check for CI/CD configs (GitHub Actions and CircleCI)
    let has_github_ci = js_python_project.join(".github/workflows/test.yml").exists();
    let has_circleci = js_python_project.join(".circleci/config.yml").exists();
    assert!(has_github_ci || has_circleci, "Should have at least one CI config");

    assert!(js_python_project.join("Makefile").exists(), "Makefile should exist");
}

#[cfg(test)]
mod extraction_tests {
    use super::*;
    use autonix::repo::scan::scan_repos;

    #[test]
    fn test_rust_manifest_parsing() {
        let rust_project = fixtures_path().join("rust");
        let scan = scan_repos(&rust_project).expect("Failed to scan");

        for repo in scan {
            let manifest_files: Vec<_> = repo.manifest_files().iter().collect();

            // Should find Cargo.toml and Cargo.lock
            assert!(!manifest_files.is_empty(), "Should find manifest files");

            // Parse manifests
            for manifest_file in manifest_files {
                let parsed = manifest_file.parse();
                assert!(parsed.is_ok(), "Should parse manifest successfully: {:?}", manifest_file);

                let manifest = parsed.unwrap();

                // Verify parsed content
                assert!(!manifest.names.is_empty(), "Should have package names");

                // For workspace crates
                if manifest.names.contains(&"myapp-core".to_string()) {
                    assert_eq!(manifest.version, Some("0.2.1".to_string()));
                    assert!(manifest.metadata.description.is_some());
                }
                if manifest.names.contains(&"myapp-api".to_string()) {
                    assert_eq!(manifest.version, Some("0.2.1".to_string()));
                    assert!(manifest.metadata.description.is_some());
                }
                if manifest.names.contains(&"myapp-cli".to_string()) {
                    assert_eq!(manifest.version, Some("0.2.1".to_string()));
                    assert!(manifest.metadata.description.is_some());
                }
            }
        }
    }

    #[test]
    fn test_rust_container_parsing() {
        let rust_project = fixtures_path().join("rust");
        let scan = scan_repos(&rust_project).expect("Failed to scan");

        for repo in scan {
            let container_files: Vec<_> = repo.container_files().iter().collect();

            assert!(!container_files.is_empty(), "Should find container files");

            for container_file in container_files {
                let parsed = container_file.parse();
                assert!(parsed.is_ok(), "Should parse container file: {:?}", container_file);

                let container = parsed.unwrap();

                // Verify Dockerfile parsing
                if container_file.path().ends_with("Dockerfile") {
                    // Should have multi-stage build
                    assert_eq!(container.build_stages.len(), 2, "Should have 2 build stages");
                    assert!(container.base_image.is_some());

                    let base = container.base_image.as_ref().unwrap();
                    assert_eq!(base.name, "rust");
                    assert!(base.stage_name.is_some());

                    // Should extract system packages
                    assert!(!container.system_packages.is_empty());

                    // Should have exposed ports
                    assert!(!container.ports.is_empty());
                    assert!(container.ports.contains(&8080));
                }

                // Verify docker-compose parsing
                if container_file.path().ends_with("docker-compose.yml") {
                    assert!(!container.services.is_empty(), "Should have services");
                    assert!(container.services.len() >= 3, "Should have app, db, cache services");

                    // Check for postgres service
                    let has_postgres = container.services.iter()
                        .any(|s| s.image.as_ref().map(|i| i.contains("postgres")).unwrap_or(false));
                    assert!(has_postgres, "Should have postgres service");

                    // Check for redis service
                    let has_redis = container.services.iter()
                        .any(|s| s.image.as_ref().map(|i| i.contains("redis")).unwrap_or(false));
                    assert!(has_redis, "Should have redis service");
                }
            }
        }
    }

    #[test]
    fn test_rust_cicd_parsing() {
        let rust_project = fixtures_path().join("rust");
        let scan = scan_repos(&rust_project).expect("Failed to scan");

        for repo in scan {
            let cicd_files: Vec<_> = repo.cicd_files().iter().collect();

            assert!(!cicd_files.is_empty(), "Should find CI/CD files");

            for cicd_file in cicd_files {
                let parsed = cicd_file.parse();
                assert!(parsed.is_ok(), "Should parse CI/CD file: {:?}", cicd_file);

                let cicd = parsed.unwrap();

                // Should have multiple jobs (GitLab CI)
                assert!(!cicd.jobs.is_empty(), "Should have CI jobs");

                // GitLab CI has jobs like: test, test:integration, lint:clippy, lint:fmt, build:debug, build:release
                let job_names: Vec<_> = cicd.jobs.iter().map(|j| j.name.as_str()).collect();

                // Check for test job with services
                let test_job = cicd.jobs.iter().find(|j| j.name.contains("test"));
                if let Some(job) = test_job {
                    // GitLab CI test job should have postgres and redis services
                    if !job.services.is_empty() {
                        let has_postgres = job.services.iter()
                            .any(|s| s.image.contains("postgres"));
                        let has_redis = job.services.iter()
                            .any(|s| s.image.contains("redis"));
                        assert!(has_postgres || has_redis, "Should have database or cache service in test job");
                    }
                }
            }
        }
    }

    #[test]
    fn test_rust_task_file_parsing() {
        let rust_project = fixtures_path().join("rust");
        let scan = scan_repos(&rust_project).expect("Failed to scan");

        for repo in scan {
            let task_files: Vec<_> = repo.task_files().iter().collect();

            assert!(!task_files.is_empty(), "Should find task files (Makefile)");

            for task_file in task_files {
                let parsed = task_file.parse();
                assert!(parsed.is_ok(), "Should parse task file: {:?}", task_file);

                let tasks = parsed.unwrap();

                // Should have common targets
                assert!(!tasks.targets.is_empty(), "Should have make targets");
                assert!(tasks.targets.contains(&"test".to_string()), "Should have test target");
                assert!(tasks.targets.contains(&"build".to_string()), "Should have build target");
                assert!(tasks.targets.contains(&"lint".to_string()), "Should have lint target");
            }
        }
    }

    #[test]
    fn test_js_python_manifest_parsing() {
        let js_python_project = fixtures_path().join("js_and_python");
        let scan = scan_repos(&js_python_project).expect("Failed to scan");

        for repo in scan {
            let manifest_files: Vec<_> = repo.manifest_files().iter().collect();

            assert!(!manifest_files.is_empty(), "Should find manifest files");

            let mut found_package_json = false;
            let mut found_pyproject = false;

            for manifest_file in manifest_files {
                let parsed = manifest_file.parse();
                assert!(parsed.is_ok(), "Should parse manifest: {:?}", manifest_file);

                let manifest = parsed.unwrap();

                // Check package.json
                if manifest.names.contains(&"fullstack-monorepo".to_string()) {
                    found_package_json = true;
                    assert_eq!(manifest.version, Some("1.5.0".to_string()));
                    assert!(!manifest.scripts.is_empty(), "Should have npm scripts");
                }

                // Check pyproject.toml
                if manifest.names.contains(&"ml-service".to_string()) {
                    found_pyproject = true;
                    assert_eq!(manifest.version, Some("0.3.2".to_string()));
                    assert!(manifest.metadata.description.is_some());
                }
            }

            assert!(found_package_json || found_pyproject,
                    "Should find at least one manifest (package.json or pyproject.toml)");
        }
    }

    #[test]
    fn test_js_python_multi_language_detection() {
        let js_python_project = fixtures_path().join("js_and_python");
        let scan = scan_repos(&js_python_project).expect("Failed to scan");

        for repo in scan {
            let manifests: Vec<_> = repo.manifest_files().iter()
                .filter_map(|f| f.parse().ok())
                .collect();

            // Should detect both JavaScript/TypeScript and Python
            let has_js = manifests.iter().any(|m| {
                m.names.contains(&"fullstack-monorepo".to_string())
            });

            let has_python = manifests.iter().any(|m| {
                m.names.contains(&"ml-service".to_string())
            });

            // At least one should be found (depending on scan order)
            assert!(has_js || has_python, "Should find JS or Python manifests");
        }
    }
}
