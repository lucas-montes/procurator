use std::process::{Command, Output};
use std::path::{Path, PathBuf};
use std::fs;
use std::env;
use std::marker::PhantomData;

#[derive(Debug)]
pub enum BuildError {
    CommandFailed(String),
    IoError(std::io::Error),
    InvalidPath(String),
    TempDirError(String),
    StateValidationFailed(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::CommandFailed(msg) => write!(f, "Command failed: {}", msg),
            BuildError::IoError(err) => write!(f, "IO error: {}", err),
            BuildError::InvalidPath(path) => write!(f, "Invalid path: {}", path),
            BuildError::TempDirError(msg) => write!(f, "Temp directory error: {}", msg),
            BuildError::StateValidationFailed(msg) => write!(f, "State validation failed: {}", msg),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<std::io::Error> for BuildError {
    fn from(err: std::io::Error) -> Self {
        BuildError::IoError(err)
    }
}

// Typestate markers
pub struct Initial;
pub struct Built;
pub struct Tested;
pub struct StateSaved;

#[derive(Debug)]
pub struct BuildResult {
    pub build_output: Output,
    pub temp_dir: PathBuf,
}

#[derive(Debug)]
pub struct TestResult {
    pub build_result: BuildResult,
    pub test_output: Output,
}

#[derive(Debug)]
pub struct StateResult {
    pub test_result: TestResult,
    pub state_changed: bool,
    pub new_state_path: Option<PathBuf>,
}

pub struct Builder<State> {
    flake_content: String,
    additional_files: Vec<(String, String)>,
    _state: PhantomData<State>,
}

impl Builder<Initial> {
    pub fn new(flake_content: String) -> Self {
        Self {
            flake_content,
            additional_files: Vec::new(),
            _state: PhantomData,
        }
    }

    pub fn with_files(mut self, files: Vec<(String, String)>) -> Self {
        self.additional_files = files;
        self
    }

    /// Create a temporary directory with the flake content
    fn create_temp_flake(&self) -> Result<PathBuf, BuildError> {
        let temp_base = env::temp_dir();
        let temp_name = format!("procurator_build_{}", std::process::id());
        let temp_dir = temp_base.join(temp_name);

        // Remove existing temp dir if it exists
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        fs::create_dir_all(&temp_dir)
            .map_err(|e| BuildError::TempDirError(format!("Failed to create temp dir: {}", e)))?;

        let flake_path = temp_dir.join("flake.nix");
        fs::write(&flake_path, &self.flake_content)?;

        // Write additional files
        for (filename, content) in &self.additional_files {
            let file_path = temp_dir.join(filename);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, content)?;
        }

        Ok(temp_dir)
    }

    pub fn build(self) -> Result<Builder<Built>, BuildError> {
        let temp_dir = self.create_temp_flake()?;

        let output = Command::new("nix")
            .args(["build"])
            .current_dir(&temp_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Clean up on failure
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(BuildError::CommandFailed(format!("nix build failed: {}", stderr)));
        }

        Ok(Builder {
            flake_content: self.flake_content,
            additional_files: self.additional_files,
            _state: PhantomData,
        })
    }

    pub fn build_package(self, package: &str) -> Result<Builder<Built>, BuildError> {
        let temp_dir = self.create_temp_flake()?;

        let package_ref = format!(".#{}", package);
        let output = Command::new("nix")
            .args(["build", &package_ref])
            .current_dir(&temp_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Clean up on failure
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(BuildError::CommandFailed(format!("nix build {} failed: {}", package, stderr)));
        }

        Ok(Builder {
            flake_content: self.flake_content,
            additional_files: self.additional_files,
            _state: PhantomData,
        })
    }
}

impl Builder<Built> {
    pub fn test(self) -> Result<Builder<Tested>, BuildError> {
        let temp_dir = self.create_temp_flake()?;

        let check_output = Command::new("nix")
            .args(["flake", "check"])
            .current_dir(&temp_dir)
            .output()?;

        if !check_output.status.success() {
            let stderr = String::from_utf8_lossy(&check_output.stderr);
            // Clean up on failure
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(BuildError::CommandFailed(format!("nix flake check failed: {}", stderr)));
        }

        Ok(Builder {
            flake_content: self.flake_content,
            additional_files: self.additional_files,
            _state: PhantomData,
        })
    }

    fn create_temp_flake(&self) -> Result<PathBuf, BuildError> {
        let temp_base = env::temp_dir();
        let temp_name = format!("procurator_build_{}", std::process::id());
        let temp_dir = temp_base.join(temp_name);

        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        fs::create_dir_all(&temp_dir)
            .map_err(|e| BuildError::TempDirError(format!("Failed to create temp dir: {}", e)))?;

        let flake_path = temp_dir.join("flake.nix");
        fs::write(&flake_path, &self.flake_content)?;

        for (filename, content) in &self.additional_files {
            let file_path = temp_dir.join(filename);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, content)?;
        }

        Ok(temp_dir)
    }
}

impl Builder<Tested> {
    pub fn save_state(self, current_state_path: &str) -> Result<Builder<StateSaved>, BuildError> {
        // Generate new state from the flake
        let temp_dir = self.create_temp_flake()?;

        let state_output = Command::new("nix")
            .args(["build", ".#state"])
            .current_dir(&temp_dir)
            .output()?;

        if !state_output.status.success() {
            let stderr = String::from_utf8_lossy(&state_output.stderr);
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(BuildError::CommandFailed(format!("Failed to generate state: {}", stderr)));
        }

        // Check if state has changed
        let new_state_file = temp_dir.join("result/state.json");
        let state_changed = if Path::new(current_state_path).exists() && new_state_file.exists() {
            let current_state = fs::read_to_string(current_state_path)?;
            let new_state = fs::read_to_string(&new_state_file)?;
            current_state != new_state
        } else {
            true // State changed if files don't exist or are different
        };

        // Clean up temp directory
        let _ = fs::remove_dir_all(&temp_dir);

        Ok(Builder {
            flake_content: self.flake_content,
            additional_files: self.additional_files,
            _state: PhantomData,
        })
    }

    fn create_temp_flake(&self) -> Result<PathBuf, BuildError> {
        let temp_base = env::temp_dir();
        let temp_name = format!("procurator_build_{}", std::process::id());
        let temp_dir = temp_base.join(temp_name);

        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        fs::create_dir_all(&temp_dir)
            .map_err(|e| BuildError::TempDirError(format!("Failed to create temp dir: {}", e)))?;

        let flake_path = temp_dir.join("flake.nix");
        fs::write(&flake_path, &self.flake_content)?;

        for (filename, content) in &self.additional_files {
            let file_path = temp_dir.join(filename);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, content)?;
        }

        Ok(temp_dir)
    }
}

impl Builder<StateSaved> {
    pub fn finalize(self) -> Result<(), BuildError> {
        // Final cleanup and validation
        Ok(())
    }
}
