use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::recording::StartReq;
use anyhow::Result;
use tokio::{
    fs,
    sync::{Mutex, oneshot},
};

#[derive(Clone)]
pub struct AppState {
    pub pending_dir: PathBuf,
    pub finished_dir: PathBuf,
    pub manager: Arc<RecordingManager>,
}

pub struct RecordingManager {
    // name -> control
    inner: Mutex<HashMap<String, RecordingControl>>,
    persist_path: PathBuf,
}

struct RecordingControl {
    stop: Option<oneshot::Sender<()>>,
    req: StartReq,
}

impl RecordingManager {
    pub fn new(persist_path: PathBuf) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            persist_path,
        }
    }

    async fn save(&self, map: &HashMap<String, RecordingControl>) -> Result<()> {
        let list: Vec<&StartReq> = map.values().map(|c| &c.req).collect();
        let json = serde_json::to_string(&list)?;
        if let Some(parent) = self.persist_path.parent() {
            fs::create_dir_all(parent).await.ok();
        }
        fs::write(&self.persist_path, json).await?;
        Ok(())
    }

    pub async fn load(&self) -> Result<Vec<StartReq>> {
        match fs::read_to_string(&self.persist_path).await {
            Ok(content) => Ok(serde_json::from_str(&content)?),
            Err(_) => Ok(Vec::new()),
        }
    }

    pub async fn start(&self, req: StartReq, stop: oneshot::Sender<()>) -> Result<()> {
        let mut map = self.inner.lock().await;
        if map.contains_key(&req.name) {
            anyhow::bail!("Recording '{}' is already running", req.name);
        }
        map.insert(
            req.name.clone(),
            RecordingControl {
                stop: Some(stop),
                req,
            },
        );
        self.save(&map).await
    }

    pub async fn stop(&self, name: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        if let Some(mut ctrl) = map.remove(name) {
            if let Some(tx) = ctrl.stop.take() {
                let _ = tx.send(());
            }
            self.save(&map).await?;
        }
        Ok(())
    }

    pub async fn finish(&self, name: &str) {
        let mut map = self.inner.lock().await;
        if map.remove(name).is_some() {
            let _ = self.save(&map).await;
        }
    }

    pub async fn is_running(&self, name: &str) -> bool {
        let map = self.inner.lock().await;
        map.contains_key(name)
    }
}
