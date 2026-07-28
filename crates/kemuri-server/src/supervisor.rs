use std::collections::HashMap;
use tokio::task::JoinHandle;

pub struct Supervisor {
    tasks: HashMap<&'static str, JoinHandle<()>>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn spawn(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.tasks.insert(name, handle);
    }

    pub async fn check(&mut self) -> Vec<&'static str> {
        let mut failed = Vec::new();
        let mut to_restart = Vec::new();

        for (name, handle) in &self.tasks {
            if handle.is_finished() {
                failed.push(*name);
            }
        }

        for name in &failed {
            if let Some(handle) = self.tasks.remove(*name) {
                match handle.await {
                    Ok(()) => {
                        tracing::warn!(task = name, "task completed unexpectedly");
                    }
                    Err(e) => {
                        if e.is_panic() {
                            tracing::error!(task = name, "task panicked");
                        } else {
                            tracing::warn!(task = name, "task cancelled");
                        }
                    }
                }
                to_restart.push(*name);
            }
        }

        to_restart
    }

    pub async fn shutdown(self) {
        for (name, handle) in self.tasks {
            handle.abort();
            tracing::debug!(task = name, "task aborted");
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}
