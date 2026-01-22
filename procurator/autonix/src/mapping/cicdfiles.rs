use std::path::Path;
use std::collections::HashMap;

use serde::Deserialize;
use crate::mapping::{ParseError, Parseable};


/// CI/CD configuration files
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum CiCdFile {
    // GitHub Actions
    GitHubActions,

    // GitLab CI
    GitLabCI,

    // CircleCI
    CircleCI,

    // Travis CI
    TravisCI,

    // Jenkins
    Jenkinsfile,

    // Azure Pipelines
    AzurePipelines,

    // Bitbucket Pipelines
    BitbucketPipelines,

    // Drone CI
    DroneCI,

    // Buildkite
    Buildkite,

    // AppVeyor
    AppVeyor,

    // Wercker
    Wercker,
}

/// A CI/CD job or workflow
#[derive(Debug, Clone, Default)]
pub struct CiJob {
    pub name: String,
    pub steps: Vec<CiStep>,
    pub services: Vec<CiService>,
    pub env: HashMap<String, String>,
}

/// A single step in a CI job
#[derive(Debug, Clone)]
pub struct CiStep {
    pub name: Option<String>,
    pub run: Option<String>,
}

/// A service dependency (database, cache, etc.)
#[derive(Debug, Clone)]
pub struct CiService {
    pub name: String,
    pub image: String,  // e.g., "postgres:15"
    pub env: HashMap<String, String>,
}

/// Parsed CI/CD file information
#[derive(Debug, Clone, Default)]
pub struct ParsedCiCdFile {
    /// Jobs/workflows defined
    pub jobs: Vec<CiJob>,

    /// Global environment variables
    pub env: HashMap<String, String>,
}

// GitHub Actions YAML structures (minimal subset)
#[derive(Deserialize)]
struct GitHubWorkflow {
    #[serde(default)]
    env: HashMap<String, serde_yaml_ng::Value>,

    #[serde(default)]
    jobs: HashMap<String, GitHubJob>,
}

#[derive(Deserialize)]
struct GitHubJob {
    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    steps: Vec<GitHubStep>,

    #[serde(default)]
    services: HashMap<String, GitHubService>,

    #[serde(default)]
    env: HashMap<String, serde_yaml_ng::Value>,
}

#[derive(Deserialize)]
struct GitHubStep {
    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    run: Option<String>,
}

#[derive(Deserialize)]
struct GitHubService {
    image: String,

    #[serde(default)]
    env: HashMap<String, serde_yaml_ng::Value>,
}

impl Parseable for CiCdFile {
    type Output = ParsedCiCdFile;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        let content = std::fs::read_to_string(path)?;

        match self {
            Self::GitHubActions => parse_github_actions(&content),

            // Other CI systems not yet implemented
            Self::GitLabCI
            | Self::CircleCI
            | Self::TravisCI
            | Self::Jenkinsfile
            | Self::AzurePipelines
            | Self::BitbucketPipelines
            | Self::DroneCI
            | Self::Buildkite
            | Self::AppVeyor
            | Self::Wercker => Ok(ParsedCiCdFile::default()),
        }
    }
}

/// Parse GitHub Actions workflow file
fn parse_github_actions(content: &str) -> Result<ParsedCiCdFile, ParseError> {
    let workflow: GitHubWorkflow = serde_yaml_ng::from_str(content)
        .map_err(|e| ParseError::InvalidFormat(format!("GitHub Actions parse error: {}", e)))?;

    let mut result = ParsedCiCdFile {
        jobs: Vec::new(),
        env: extract_env_map(&workflow.env),
    };

    for (job_id, job) in workflow.jobs {
        let mut ci_job = CiJob {
            name: job.name.unwrap_or(job_id),
            steps: Vec::new(),
            services: Vec::new(),
            env: extract_env_map(&job.env),
        };

        // Extract steps
        for step in job.steps {
            if let Some(run) = step.run {
                ci_job.steps.push(CiStep {
                    name: step.name,
                    run: Some(run),
                });
            }
        }

        // Extract services
        for (service_name, service) in job.services {
            ci_job.services.push(CiService {
                name: service_name,
                image: service.image,
                env: extract_env_map(&service.env),
            });
        }

        result.jobs.push(ci_job);
    }

    Ok(result)
}

/// Helper to convert YAML value map to string map
fn extract_env_map(yaml_map: &HashMap<String, serde_yaml_ng::Value>) -> HashMap<String, String> {
    yaml_map
        .iter()
        .filter_map(|(k, v)| {
            match v {
                serde_yaml_ng::Value::String(s) => Some((k.clone(), s.clone())),
                serde_yaml_ng::Value::Number(n) => Some((k.clone(), n.to_string())),
                serde_yaml_ng::Value::Bool(b) => Some((k.clone(), b.to_string())),
                _ => None, // Skip complex values, secrets, expressions
            }
        })
        .collect()
}

impl TryFrom<&str> for CiCdFile {
    type Error = ();

    fn try_from(path: &str) -> Result<Self, Self::Error> {
        let is_yaml = path.ends_with(".yml") || path.ends_with(".yaml");
        // GitHub Actions - .github/workflows/*.yml or *.yaml
        if path.contains(".github/workflows/") && is_yaml {
            return Ok(Self::GitHubActions);
        }

        // CircleCI - .circleci/config.yml or any yml in .circleci/
        if path.contains(".circleci/") && is_yaml {
            return Ok(Self::CircleCI);
        }

        // Buildkite - .buildkite/ directory
        if path.contains(".buildkite/") && is_yaml {
            return Ok(Self::Buildkite);
        }

        // Jenkins - contains Jenkinsfile anywhere in path
        if path.contains("Jenkinsfile") {
            return Ok(Self::Jenkinsfile);
        }

        // Simple filename/path suffix matches
        match path {
            // Exact filename matches
            ".gitlab-ci.yml" | ".gitlab-ci.yaml" => Ok(Self::GitLabCI),
            ".travis.yml" => Ok(Self::TravisCI),
            "Jenkinsfile" => Ok(Self::Jenkinsfile),
            "azure-pipelines.yml" | "azure-pipelines.yaml" => Ok(Self::AzurePipelines),
            "bitbucket-pipelines.yml" => Ok(Self::BitbucketPipelines),
            ".drone.yml" | ".drone.yaml" => Ok(Self::DroneCI),
            "buildkite.yml" | "buildkite.yaml" => Ok(Self::Buildkite),
            "appveyor.yml" | ".appveyor.yml" => Ok(Self::AppVeyor),
            "wercker.yml" => Ok(Self::Wercker),

            // Path suffix matches
            _ if path.ends_with(".gitlab-ci.yml") || path.ends_with(".gitlab-ci.yaml") => {
                Ok(Self::GitLabCI)
            }
            _ if path.ends_with(".travis.yml") => Ok(Self::TravisCI),
            _ if path.ends_with("azure-pipelines.yml")
                || path.ends_with("azure-pipelines.yaml") =>
            {
                Ok(Self::AzurePipelines)
            }
            _ if path.ends_with("bitbucket-pipelines.yml") => Ok(Self::BitbucketPipelines),
            _ if path.ends_with(".drone.yml") || path.ends_with(".drone.yaml") => Ok(Self::DroneCI),
            _ if path.ends_with("buildkite.yml") || path.ends_with("buildkite.yaml") => {
                Ok(Self::Buildkite)
            }
            _ if path.ends_with("appveyor.yml") || path.ends_with(".appveyor.yml") => {
                Ok(Self::AppVeyor)
            }
            _ if path.ends_with("wercker.yml") => Ok(Self::Wercker),
            _ if path.ends_with("Jenkinsfile") => Ok(Self::Jenkinsfile),

            _ => Err(()),
        }
    }
}
