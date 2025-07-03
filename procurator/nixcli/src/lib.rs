use std::path::{Path, PathBuf};

use tokio::{fs, process::Command};

#[derive(Debug, Clone)]
pub enum NixError {
    CommandFailed {
        command: String,
        exit_code: i32,
        stderr: String
    },
    IoError(String),
    ParseError(String),
}

impl std::fmt::Display for NixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NixError::CommandFailed { command, exit_code, stderr } => {
                write!(f, "Nix command '{}' failed with exit code {}: {}", command, exit_code, stderr)
            }
            NixError::IoError(msg) => write!(f, "IO error: {}", msg),
            NixError::ParseError(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl std::error::Error for NixError {}

impl From<std::io::Error> for NixError {
    fn from(err: std::io::Error) -> Self {
        NixError::IoError(err.to_string())
    }
}

#[derive(Debug)]
pub struct NixOutput<T> {
    pub result: T,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl<T> NixOutput<T> {
    fn new(result: T, stdout: String, stderr: String, exit_code: i32) -> Self {
        Self { result, stdout, stderr, exit_code }
    }
}

pub struct NixCli {
    binary: String,
    working_dir: Option<PathBuf>,
}

impl NixCli {
    pub fn new() -> Self {
        Self {
            binary: "nix".to_string(),
            working_dir: None,
        }
    }

    pub fn with_working_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    async fn execute(&self, args: &[&str]) -> Result<(String, String, i32), NixError> {
        let mut command = Command::new(&self.binary);
        command.args(args);

        if let Some(ref dir) = self.working_dir {
            command.current_dir(dir);
        }

        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let output = command.output().await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok((stdout, stderr, exit_code))
    }

    pub async fn build(&self, args: &BuildArgs) -> Result<NixOutput<BuildResult>, NixError> {
        let mut cmd_args = vec!["build"];

        // Flake reference
        cmd_args.push(&args.flake_ref);

        // Options
        if args.no_link {
            cmd_args.push("--no-link");
        }
        if args.dry_run {
            cmd_args.push("--dry-run");
        }
        if let Some(ref out_link) = args.out_link {
            cmd_args.push("--out-link");
            cmd_args.push(out_link);
        }

        // Extra args
        let extra_refs: Vec<&str> = args.extra_args.iter().map(|s| s.as_str()).collect();
        cmd_args.extend(extra_refs);

        let (stdout, stderr, exit_code) = self.execute(&cmd_args).await?;

        if exit_code != 0 {
            return Err(NixError::CommandFailed {
                command: format!("nix {}", cmd_args.join(" ")),
                exit_code,
                stderr: stderr.clone(),
            });
        }

        let result = BuildResult {
            store_path: self.parse_store_path(&stdout),
            out_link: if args.no_link { None } else { args.out_link.clone().or_else(|| Some("result".to_string())) },
        };

        Ok(NixOutput::new(result, stdout, stderr, exit_code))
    }

    pub async fn flake_check(&self, args: &FlakeCheckArgs) -> Result<NixOutput<FlakeCheckResult>, NixError> {
        let mut cmd_args = vec!["flake", "check"];

        cmd_args.push(&args.flake_ref);

        if args.all_systems {
            cmd_args.push("--all-systems");
        }
        if args.no_build {
            cmd_args.push("--no-build");
        }

        let extra_refs: Vec<&str> = args.extra_args.iter().map(|s| s.as_str()).collect();
        cmd_args.extend(extra_refs);

        let (stdout, stderr, exit_code) = self.execute(&cmd_args).await?;

        if exit_code != 0 {
            return Err(NixError::CommandFailed {
                command: format!("nix {}", cmd_args.join(" ")),
                exit_code,
                stderr: stderr.clone(),
            });
        }

        let result = FlakeCheckResult {
            checks_passed: exit_code == 0 && !stderr.contains("error:"),
        };

        Ok(NixOutput::new(result, stdout, stderr, exit_code))
    }

    pub async fn instantiate(&self, args: &InstantiateArgs) -> Result<NixOutput<InstantiateResult>, NixError> {
        let mut cmd_args = vec!["instantiate"];

        cmd_args.push(&args.flake_ref);

        if let Some(ref attr) = args.attribute {
            cmd_args.push("--attr");
            cmd_args.push(attr);
        }
        if let Some(ref expr) = args.expr {
            cmd_args.push("--expr");
            cmd_args.push(expr);
        }
        if args.eval_only {
            cmd_args.push("--eval-only");
        }
        if args.json {
            cmd_args.push("--json");
        }

        let extra_refs: Vec<&str> = args.extra_args.iter().map(|s| s.as_str()).collect();
        cmd_args.extend(extra_refs);

        let (stdout, stderr, exit_code) = self.execute(&cmd_args).await?;

        if exit_code != 0 {
            return Err(NixError::CommandFailed {
                command: format!("nix {}", cmd_args.join(" ")),
                exit_code,
                stderr: stderr.clone(),
            });
        }

        let result = InstantiateResult {
            value: if stdout.trim().is_empty() { None } else { Some(stdout.trim().to_string()) },
        };

        Ok(NixOutput::new(result, stdout, stderr, exit_code))
    }

    pub async fn eval(&self, flake_ref: &str, attribute: Option<&str>) -> Result<NixOutput<InstantiateResult>, NixError> {
        let mut cmd_args = vec!["eval"];

        let target = match attribute {
            Some(attr) => format!("{}#{}", flake_ref, attr),
            None => flake_ref.to_string(),
        };
        cmd_args.push(&target);

        let (stdout, stderr, exit_code) = self.execute(&cmd_args).await?;

        if exit_code != 0 {
            return Err(NixError::CommandFailed {
                command: format!("nix {}", cmd_args.join(" ")),
                exit_code,
                stderr: stderr.clone(),
            });
        }

        let result = InstantiateResult {
            value: if stdout.trim().is_empty() { None } else { Some(stdout.trim().to_string()) },
        };

        Ok(NixOutput::new(result, stdout, stderr, exit_code))
    }

    pub async fn eval_json(&self, flake_ref: &str, attribute: Option<&str>) -> Result<NixOutput<InstantiateResult>, NixError> {
        let mut cmd_args = vec!["eval", "--json"];

        let target = match attribute {
            Some(attr) => format!("{}#{}", flake_ref, attr),
            None => flake_ref.to_string(),
        };
        cmd_args.push(&target);

        let (stdout, stderr, exit_code) = self.execute(&cmd_args).await?;

        if exit_code != 0 {
            return Err(NixError::CommandFailed {
                command: format!("nix {}", cmd_args.join(" ")),
                exit_code,
                stderr: stderr.clone(),
            });
        }

        let result = InstantiateResult {
            value: if stdout.trim().is_empty() { None } else { Some(stdout.trim().to_string()) },
        };

        Ok(NixOutput::new(result, stdout, stderr, exit_code))
    }

    pub async fn build_from_content(
        &self,
        flake_content: &str,
        additional_files: &[(&str, &str)],
        package: Option<&str>
    ) -> Result<NixOutput<BuildResult>, NixError> {
        // Create temporary directory
        let temp_dir = self.create_temp_workspace(flake_content, additional_files).await?;

        // Build the package
        let flake_ref = match package {
            Some(pkg) => format!(".#{}", pkg),
            None => ".".to_string(),
        };

        let build_args = BuildArgs::new(&flake_ref).no_link();

        // Create a new NixCli instance with the temp directory
        let temp_nix = Self::new().with_working_dir(&temp_dir);
        let result = temp_nix.build(&build_args).await;

        // Clean up temp directory
        let _ = fs::remove_dir_all(&temp_dir).await;

        result
    }

    pub async fn flake_check_from_content(
        &self,
        flake_content: &str,
        additional_files: &[(&str, &str)]
    ) -> Result<NixOutput<FlakeCheckResult>, NixError> {
        // Create temporary directory
        let temp_dir = self.create_temp_workspace(flake_content, additional_files).await?;

        let check_args = FlakeCheckArgs::new(".");

        // Create a new NixCli instance with the temp directory
        let temp_nix = Self::new().with_working_dir(&temp_dir);
        let result = temp_nix.flake_check(&check_args).await;

        // Clean up temp directory
        let _ = fs::remove_dir_all(&temp_dir).await;

        result
    }

    async fn create_temp_workspace(
        &self,
        flake_content: &str,
        additional_files: &[(&str, &str)]
    ) -> Result<PathBuf, NixError> {
        use std::env;

        let temp_base = env::temp_dir();
        let temp_name = format!("nixcli_workspace_{}", std::process::id());
        let temp_dir = temp_base.join(temp_name);

        // Remove existing temp dir if it exists
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).await?;
        }

        // Create temp directory
        fs::create_dir_all(&temp_dir).await?;

        // Write flake.nix
        let flake_path = temp_dir.join("flake.nix");
        fs::write(&flake_path, flake_content).await?;

        // Write additional files
        for (filename, content) in additional_files {
            let file_path = temp_dir.join(filename);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&file_path, content).await?;
        }

        Ok(temp_dir)
    }

    fn parse_store_path(&self, stdout: &str) -> Option<String> {
        for line in stdout.lines() {
            if line.starts_with("/nix/store/") {
                return Some(line.trim().to_string());
            }
        }
        None
    }
}

impl Default for NixCli {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct BuildArgs {
    pub flake_ref: String,
    pub no_link: bool,
    pub dry_run: bool,
    pub out_link: Option<String>,
    pub extra_args: Vec<String>,
}

impl BuildArgs {
    pub fn new(flake_ref: impl Into<String>) -> Self {
        Self {
            flake_ref: flake_ref.into(),
            no_link: false,
            dry_run: false,
            out_link: None,
            extra_args: Vec::new(),
        }
    }

    pub fn no_link(mut self) -> Self {
        self.no_link = true;
        self
    }

    pub fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }

    pub fn out_link(mut self, path: impl Into<String>) -> Self {
        self.out_link = Some(path.into());
        self
    }

    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct FlakeCheckArgs {
    pub flake_ref: String,
    pub all_systems: bool,
    pub no_build: bool,
    pub extra_args: Vec<String>,
}

impl FlakeCheckArgs {
    pub fn new(flake_ref: impl Into<String>) -> Self {
        Self {
            flake_ref: flake_ref.into(),
            all_systems: false,
            no_build: false,
            extra_args: Vec::new(),
        }
    }

    pub fn all_systems(mut self) -> Self {
        self.all_systems = true;
        self
    }

    pub fn no_build(mut self) -> Self {
        self.no_build = true;
        self
    }

    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct InstantiateArgs {
    pub flake_ref: String,
    pub attribute: Option<String>,
    pub expr: Option<String>,
    pub eval_only: bool,
    pub json: bool,
    pub extra_args: Vec<String>,
}

impl InstantiateArgs {
    pub fn new(flake_ref: impl Into<String>) -> Self {
        Self {
            flake_ref: flake_ref.into(),
            attribute: None,
            expr: None,
            eval_only: false,
            json: false,
            extra_args: Vec::new(),
        }
    }

    pub fn attribute(mut self, attr: impl Into<String>) -> Self {
        self.attribute = Some(attr.into());
        self
    }

    pub fn expr(mut self, expr: impl Into<String>) -> Self {
        self.expr = Some(expr.into());
        self
    }

    pub fn eval_only(mut self) -> Self {
        self.eval_only = true;
        self
    }

    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }
}

#[derive(Debug)]
pub struct BuildResult {
    pub store_path: Option<String>,
    pub out_link: Option<String>,
}

#[derive(Debug)]
pub struct FlakeCheckResult {
    pub checks_passed: bool,
}

#[derive(Debug)]
pub struct InstantiateResult {
    pub value: Option<String>,
}
