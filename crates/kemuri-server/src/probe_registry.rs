use std::collections::HashMap;
use std::sync::Arc;

use kemuri_core::ProbeKind;
use kemuri_probes::Probe;

pub struct ProbeRegistry {
    probes: HashMap<ProbeKind, Arc<dyn Probe>>,
}

impl ProbeRegistry {
    pub fn new() -> Self {
        Self {
            probes: HashMap::new(),
        }
    }

    pub fn register(&mut self, probe: Arc<dyn Probe>) {
        self.probes.insert(probe.kind(), probe);
    }

    pub fn get(&self, kind: ProbeKind) -> Option<Arc<dyn Probe>> {
        self.probes.get(&kind).cloned()
    }
}

impl Default for ProbeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
