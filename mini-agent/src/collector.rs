use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Client;
use sysinfo::{Disks, System};

use crate::models::{AgentState, ProcessInfo, SystemMetrics};
use crate::ollama::collect_llm_metrics;

const PROCESS_WINDOW_MS: u128 = 5 * 60 * 1000;
const TOP_PROCESS_COUNT: usize = 5;

#[derive(Debug, Clone)]
struct ProcessSample {
    ts_ms: u128,
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Debug, Clone)]
struct ProcessHistoryEntry {
    pid: u32,
    name: String,
    samples: VecDeque<ProcessSample>,
}

type ProcessHistory = HashMap<String, ProcessHistoryEntry>;

pub struct SnapshotCollector {
    system: System,
    process_history: ProcessHistory,
}

impl SnapshotCollector {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
            process_history: HashMap::new(),
        }
    }

    pub async fn collect_snapshot(
        &mut self,
        seq: u64,
        ollama_ps_url: &str,
        client: &Client,
        watch_dirs: &[String],
        events: &Arc<Mutex<VecDeque<String>>>,
    ) -> AgentState {
        let ts_ms = now_ms();
        self.system.refresh_all();

        let cpu_percent = self.system.global_cpu_usage();
        let memory_total_bytes = self.system.total_memory();
        let memory_used_bytes = self.system.used_memory();
        let swap_total_bytes = self.system.total_swap();
        let swap_used_bytes = self.system.used_swap();

        update_process_history(&self.system, &mut self.process_history, ts_ms);
        let top_processes = compute_top_processes(&self.process_history, ts_ms);

        let (root_total_bytes, root_used_bytes) = root_disk_usage();

        let llm = collect_llm_metrics(ollama_ps_url, client).await;

        AgentState {
            seq,
            ts_ms,
            hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
            watched_dirs: watch_dirs.to_vec(),
            recent_file_events: read_recent_events(events, 20),
            system: SystemMetrics {
                cpu_percent,
                memory_used_bytes,
                memory_total_bytes,
                swap_used_bytes,
                swap_total_bytes,
                root_used_bytes,
                root_total_bytes,
                top_processes,
            },
            llm,
        }
    }
}

fn update_process_history(system: &System, history: &mut ProcessHistory, ts_ms: u128) {
    for (pid, proc_) in system.processes() {
        let key = process_key(pid.as_u32(), proc_.start_time());
        let entry = history.entry(key).or_insert_with(|| ProcessHistoryEntry {
            pid: pid.as_u32(),
            name: proc_.name().to_string_lossy().into_owned(),
            samples: VecDeque::new(),
        });

        entry.pid = pid.as_u32();
        entry.name = proc_.name().to_string_lossy().into_owned();
        entry.samples.push_back(ProcessSample {
            ts_ms,
            cpu_percent: proc_.cpu_usage(),
            memory_bytes: proc_.memory(),
        });

        prune_old_samples(&mut entry.samples, ts_ms);
    }

    history.retain(|_, entry| {
        prune_old_samples(&mut entry.samples, ts_ms);
        !entry.samples.is_empty()
    });
}

fn compute_top_processes(history: &ProcessHistory, ts_ms: u128) -> Vec<ProcessInfo> {
    let mut out: Vec<ProcessInfo> = history
        .values()
        .filter_map(|entry| {
            let valid_samples: Vec<&ProcessSample> = entry
                .samples
                .iter()
                .filter(|s| ts_ms.saturating_sub(s.ts_ms) <= PROCESS_WINDOW_MS)
                .collect();

            if valid_samples.is_empty() {
                return None;
            }

            let sum_cpu: f32 = valid_samples.iter().map(|s| s.cpu_percent).sum();
            let avg_cpu = sum_cpu / valid_samples.len() as f32;
            let current = valid_samples.last().copied()?;

            Some(ProcessInfo {
                pid: entry.pid,
                name: entry.name.clone(),
                cpu_percent: avg_cpu,
                current_cpu_percent: current.cpu_percent,
                memory_bytes: current.memory_bytes,
                samples_5m: valid_samples.len() as u32,
            })
        })
        .collect();

    out.sort_by(|a, b| {
        b.cpu_percent
            .total_cmp(&a.cpu_percent)
            .then_with(|| b.current_cpu_percent.total_cmp(&a.current_cpu_percent))
    });
    out.truncate(TOP_PROCESS_COUNT);
    out
}

fn prune_old_samples(samples: &mut VecDeque<ProcessSample>, now_ms: u128) {
    while samples
        .front()
        .map(|s| now_ms.saturating_sub(s.ts_ms) > PROCESS_WINDOW_MS)
        .unwrap_or(false)
    {
        samples.pop_front();
    }
}

fn process_key(pid: u32, start_time: u64) -> String {
    format!("{pid}:{start_time}")
}

fn root_disk_usage() -> (u64, u64) {
    let disks = Disks::new_with_refreshed_list();

    let selected = disks
        .iter()
        .find(|disk| disk.mount_point() == Path::new("/"))
        .or_else(|| disks.iter().next());

    if let Some(disk) = selected {
        let total = disk.total_space();
        let avail = disk.available_space();
        let used = total.saturating_sub(avail);
        (total, used)
    } else {
        (0, 0)
    }
}

fn read_recent_events(events: &Arc<Mutex<VecDeque<String>>>, limit: usize) -> Vec<String> {
    let queue = events.lock().expect("file event queue lock");
    queue.iter().rev().take(limit).cloned().collect::<Vec<_>>()
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
