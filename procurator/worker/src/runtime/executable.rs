use std::process::{Command, Stdio};

use cgroups_rs::{
    CgroupPid, cgroup_builder::CgroupBuilder, cpu::CpuController, memory::MemController,
};

use super::application::{Runtime, RuntimeConfig, RuntimeError, RuntimeStats};

pub struct Executable {
    control_group: cgroups_rs::Cgroup,
    start: std::time::Instant,
    config: RuntimeConfig,
    child: std::process::Child,
}

impl Executable {
    /// Create a new instance of the Executable runtime, create the cgroup and spawn the child process.
    /// This function will panic if the cgroup creation or child process spawning fails.
    /// The `config` parameter is expected to contain the necessary information to create the cgroup and spawn the child process.
    /// Check more cgroup params at: https://docs.rs/cgroups-rs/0.3.4/cgroups_rs/cgroup_builder/index.html
    fn new(config: RuntimeConfig) -> Self {
        // Acquire a handle for the cgroup hierarchy.
        let hier = cgroups_rs::hierarchies::auto();

        // Use the builder pattern (see the documentation to create the control group)
        let control_group = CgroupBuilder::new(config.name())
            .cpu()
            .shares(config.cpu_limit())
            .done()
            .memory()
            .memory_hard_limit(config.memory_limit_in_bytes() as i64)
            .done()
            .build(hier)
            .expect("failed to create cgroup");

        let child = Command::new(config.executable()) // Assume RuntimeConfig has executable() method
            .args(config.args()) // Assume args() returns Vec<String>
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn child process");

        // Add the child process to the cgroup
        let pid = CgroupPid::from(child.id() as u64);
        control_group
            .add_task(pid)
            .expect("failed to add task to cgroup");

        Self {
            control_group,
            config,
            start: std::time::Instant::now(),
            child,
        }
    }
}

impl Runtime for Executable {
    fn id(&self) -> &str {
        self.config.name()
    }

    fn start(&mut self) -> Result<(), RuntimeError> {
        // Spawn the executable
        let child = Command::new(self.config.executable()) // Assume RuntimeConfig has executable() method
            .args(self.config.args()) // Assume args() returns Vec<String>
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RuntimeError::ExecutionFailed(e.to_string()))?;

        // Add the child process to the cgroup
        let pid = CgroupPid::from(child.id() as u64);
        self.control_group
            .add_task(pid)
            .map_err(|e| RuntimeError::ExecutionFailed(e.to_string()))?;

        // Store the child process handle
        self.child = child;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), RuntimeError> {
        self.child
            .kill()
            .map_err(|e| RuntimeError::ExecutionFailed(e.to_string()))?;
        self.child
            .wait()
            .map_err(|e| RuntimeError::ExecutionFailed(e.to_string()))?;
        self.control_group
            .remove_task(CgroupPid::from(self.child.id() as u64))
            .map_err(|e| RuntimeError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    fn stats(&self) -> Result<RuntimeStats, RuntimeError> {
        let cpu: &CpuController = self
            .control_group
            .controller_of()
            .ok_or_else(|| RuntimeError::ExecutionFailed("problem with cpu stats".into()))?;
        let cpu = cpu.cpu();
        println!("CPU stats: {:?}", cpu);
        let memory: &MemController = self
            .control_group
            .controller_of()
            .ok_or_else(|| RuntimeError::ExecutionFailed("problem with memory stats".into()))?;
        let memory = memory.memory_stat();

        Ok(RuntimeStats::new(
            0.0,
            memory.usage_in_bytes,
            self.start.elapsed().as_secs(),
        ))
    }

    fn kill(&mut self) -> Result<(), RuntimeError> {
        self.stop()?;
        // Finally, clean up and delete the control group.
        self.control_group
            .delete()
            .map_err(|err| RuntimeError::ExecutionFailed(err.to_string()))

        // Note that `Cgroup` does not implement `Drop` and therefore when the
        // structure is dropped, the Cgroup will stay around. This is because, later
        // you can then re-create the `Cgroup` using `load()`. We aren't too set on
        // this behavior, so it might change in the feature. Rest assured, it will be a
        // major version change.
    }

    fn is_running(&self) -> bool {
        self.control_group.exists()
        //         match child.try_wait() {
        //     Ok(Some(status)) => println!("exited with: {status}"),
        //     Ok(None) => true,
        //     Err(e) => println!("error attempting to wait: {e}"),
        // }
    }
}
