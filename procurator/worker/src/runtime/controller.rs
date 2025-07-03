use super::application::Runtime;

type Runtimes = Box<dyn Runtime + Send + Sync>;

#[derive(Default)]
pub struct Controller {
    running: Vec<Runtimes>,
    stopped: Vec<Runtimes>,
}

impl Controller {
    pub fn add_runtime(&mut self, runtime: Runtimes) {
        self.running.push(runtime);
    }

    pub fn stop_runtime(&mut self, name: &str) {
        self.running
            .iter()
            .position(|r| r.id() == name)
            .map(|index| {
                let runtime = self.running.remove(index);
                self.stopped.push(runtime);
            });
    }

    pub fn get_running(&self) -> &Vec<Runtimes> {
        &self.running
    }

    pub fn get_stopped(&self) -> &Vec<Runtimes> {
        &self.stopped
    }
}
