use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::collector::now_ms;

pub fn start_file_watcher(
    watch_dirs: &[String],
    events: Arc<Mutex<VecDeque<String>>>,
) -> Result<RecommendedWatcher> {
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let mut queue = events.lock().expect("file event queue lock");
                let paths = event
                    .paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let line = format!("{} [{:?}] {}", now_ms(), event.kind, paths);
                queue.push_back(line);
                while queue.len() > 200 {
                    queue.pop_front();
                }
            }
        },
        Config::default(),
    )?;

    for dir in watch_dirs {
        let path = Path::new(dir);
        if path.exists() {
            if let Err(err) = watcher.watch(path, RecursiveMode::Recursive) {
                eprintln!("watch error for {}: {err}", path.display());
            }
        }
    }

    Ok(watcher)
}
