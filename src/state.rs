use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::{Mutex, oneshot};

#[derive(Clone)]
pub struct AppState {
    pub pending_dir: PathBuf,
    pub finished_dir: PathBuf,
    pub manager: Arc<RecordingManager>,
}

#[derive(Default)]
pub struct RecordingManager {
    // name -> stop channel
    inner: Mutex<HashMap<String, RecordingControl>>,
}

struct RecordingControl {
    stop: Option<oneshot::Sender<()>>,
}

impl RecordingManager {
    pub async fn start(&self, name: String, stop: oneshot::Sender<()>) -> Result<()> {
        let mut map = self.inner.lock().await;
        if map.contains_key(&name) {
            anyhow::bail!("Recording '{}' is already running", name);
        }
        map.insert(name, RecordingControl { stop: Some(stop) });
        Ok(())
    }

    pub async fn stop(&self, name: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        if let Some(mut ctrl) = map.remove(name) {
            if let Some(tx) = ctrl.stop.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }

    pub async fn finish(&self, name: &str) {
        let mut map = self.inner.lock().await;
        map.remove(name);
    }

    pub async fn is_running(&self, name: &str) -> bool {
        let map = self.inner.lock().await;
        map.contains_key(name)
    }
}
