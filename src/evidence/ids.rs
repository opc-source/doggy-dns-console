//! Deterministic evidence IDs.
//!
//! Every object referenced by an analysis finding gets a stable ID built from
//! its own data. When two objects produce the same base ID, the allocator
//! appends `#2`, `#3`, ... in first-seen order; IDs are never random.

use std::collections::HashMap;

/// Allocates evidence IDs and disambiguates duplicates.
#[derive(Debug, Clone, Default)]
pub struct IdAllocator {
    counts: HashMap<String, u32>,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `log:<ts>:<spanId|none>:<hash8>`
    pub fn log(&mut self, ts: i64, span_id: Option<&str>, hash: &str) -> String {
        let span_part = span_id.unwrap_or("none");
        self.allocate(format!("log:{ts}:{span_part}:{hash}"))
    }

    /// `span:<spanId>`
    pub fn span(&mut self, span_id: &str) -> String {
        self.allocate(format!("span:{span_id}"))
    }

    /// `plugin:<instanceId>:<order>`
    pub fn plugin(&mut self, instance_id: &str, order: usize) -> String {
        self.allocate(format!("plugin:{instance_id}:{order}"))
    }

    /// `config:<instanceId>`
    pub fn config(&mut self, instance_id: &str) -> String {
        self.allocate(format!("config:{instance_id}"))
    }

    /// `metric:<host|global>:<name>:<start>-<end>`
    pub fn metric(&mut self, scope: &str, name: &str, start: i64, end: i64) -> String {
        self.allocate(format!("metric:{scope}:{name}:{start}-{end}"))
    }

    /// `tool:<callId>:result`
    pub fn tool_result(&mut self, call_id: &str) -> String {
        self.allocate(format!("tool:{call_id}:result"))
    }

    fn allocate(&mut self, base: String) -> String {
        let count = self.counts.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{base}#{count}")
        }
    }
}
