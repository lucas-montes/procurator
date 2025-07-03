pub enum RuntimeError {
    InvalidInput(String),
    ExecutionFailed(String),
    Timeout(String),
}

pub struct RuntimeStats {
    cpu_usage: f32,
    memory_usage: u64,
    uptime: u64,
}
impl RuntimeStats {
    pub fn new(cpu_usage: f32, memory_usage: u64, uptime: u64) -> Self {
        Self {
            cpu_usage,
            memory_usage,
            uptime,
        }
    }
}

pub struct MemoryConfig {
    memory_limit: u64,
    unit: MemoryUnit,
}

pub enum MemoryUnit {
    Bytes,
    Kilobytes,
    Megabytes,
    Gigabytes,
}

pub struct RuntimeConfig {
    application: String,
    args: Vec<String>,
    name: String,
    cpu_limit: u64,
    memory_limit: MemoryConfig,
}

impl RuntimeConfig {
    pub fn new(
        application: String,
        args: Vec<String>,
        name: String,
        cpu_limit: u64,
        memory_limit: MemoryConfig,
    ) -> Self {
        Self {
            application,
            args,
            name,
            cpu_limit,
            memory_limit,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn cpu_limit(&self) -> u64 {
        self.cpu_limit
    }
    pub fn memory_limit_in_bytes(&self) -> u64 {
        match self.memory_limit.unit {
            MemoryUnit::Bytes => self.memory_limit.memory_limit,
            MemoryUnit::Kilobytes => self.memory_limit.memory_limit * 1024,
            MemoryUnit::Megabytes => self.memory_limit.memory_limit * 1024 * 1024,
            MemoryUnit::Gigabytes => self.memory_limit.memory_limit * 1024 * 1024 * 1024,
        }
    }
    pub fn executable(&self) -> &str {
        &self.application
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

pub trait Runtime {
    fn id(&self) -> &str;
    fn start(&mut self) -> Result<(), RuntimeError>;
    fn stop(&mut self) -> Result<(), RuntimeError>;
    fn stats(&self) -> Result<RuntimeStats, RuntimeError>;
    fn kill(&mut self) -> Result<(), RuntimeError>;
    fn is_running(&self) -> bool;
}
