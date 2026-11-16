//! One admission queue and shared join owners for accepted archive downloads.
use super::*;
use crate::{autosave::ArchiveWorker, media::MediaReference};
type Key = (PathBuf, i32, i64);
struct Queued {
    key: Key,
    worker: ArchiveWorker,
    reference: MediaReference,
    name: String,
}
struct OwnedTask {
    key: Option<Key>,
    abort: tokio::task::AbortHandle,
    join: AsyncMutex<Option<JoinHandle<()>>>,
}
impl OwnedTask {
    fn new(key: Option<Key>, task: JoinHandle<()>) -> Arc<Self> {
        Arc::new(Self {
            key,
            abort: task.abort_handle(),
            join: AsyncMutex::new(Some(task)),
        })
    }
    async fn wait(&self) {
        let mut join = self.join.lock().await;
        if let Some(task) = join.as_mut() {
            let _ = task.await;
            *join = None;
        }
    }
}
#[derive(Default)]
struct State {
    queued: VecDeque<Queued>,
    tasks: Vec<Arc<OwnedTask>>,
    runner: Option<Arc<OwnedTask>>,
    fenced: bool,
}
#[derive(Default)]
pub(super) struct SavedDownloads(Mutex<State>);
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    pub(crate) fn queue_saved_download(
        &self,
        archive: AutoSave,
        id: i32,
        random: i64,
        reference: MediaReference,
        name: String,
    ) -> Result<()> {
        let generation = self.generation()?;
        let key = (
            archive.folder().ok_or(Error::Storage)?.to_owned(),
            id,
            random,
        );
        let worker = archive.worker(id)?;
        let mut state = self.inner.downloads.0.lock().map_err(|_| Error::Storage)?;
        if state.fenced {
            return Err(Error::ManagerStopping);
        }
        state.tasks.retain(|task| !task.abort.is_finished());
        if state
            .tasks
            .iter()
            .any(|task| task.key.as_ref() == Some(&key))
            || state.queued.iter().any(|job| job.key == key)
        {
            return Ok(());
        }
        state.queued.push_back(Queued {
            key,
            worker,
            reference,
            name,
        });
        if state
            .runner
            .as_ref()
            .is_none_or(|task| task.abort.is_finished())
        {
            let manager = self.clone();
            state.runner = Some(OwnedTask::new(
                None,
                tokio::spawn(async move {
                    manager.run_saved_downloads(generation).await;
                }),
            ));
        }
        Ok(())
    }
    async fn run_saved_downloads(&self, generation: u64) {
        loop {
            let available = self.inner.work.changed.notified();
            tokio::pin!(available);
            available.as_mut().enable();
            match self.admit_saved_download(generation) {
                Ok(Some(true)) => {}
                Ok(Some(false)) => available.await,
                _ => return,
            }
        }
    }
    fn admit_saved_download(&self, generation: u64) -> Result<Option<bool>> {
        let mut state = self.inner.downloads.0.lock().map_err(|_| Error::Storage)?;
        if state.fenced || self.check_generation(generation).is_err() {
            return Ok(None);
        }
        if state.queued.is_empty() {
            state.runner = None;
            return Ok(None);
        }
        let Ok(permit) = self.inner.work.acquire(1) else {
            return Ok(Some(false));
        };
        let job = state.queued.pop_front().ok_or(Error::InvalidState)?;
        let key = job.key.clone();
        let manager = self.clone();
        let task = tokio::spawn(async move {
            manager.run_saved_download(job, permit, generation).await;
        });
        state.tasks.retain(|task| !task.abort.is_finished());
        state.tasks.push(OwnedTask::new(Some(key), task));
        Ok(Some(true))
    }
    async fn run_saved_download(&self, job: Queued, permit: runtime::WorkPermit, generation: u64) {
        let result = async {
            self.check_generation(generation)?;
            let stage = job.worker.stage()?;
            let path = stage.path().join("attachment.bin");
            let stage_hold: Arc<dyn Send + Sync> = stage.clone();
            let permit_hold: Arc<dyn Send + Sync> = Arc::new(permit);
            crate::media::blocking::resources(vec![stage_hold, permit_hold], async {
                tokio::select! {biased;_=job.worker.cancelled()=>return Err(Error::Paused),result=self.inner.transport.download(&job.reference,&path)=>{result?;}}
                self.check_generation(generation)?;
                job.worker.publish(job.key.2, &path, &job.name)?;
                Ok(())
            }).await
        }.await;
        if result.is_err() {
            tracing::warn!(
                chat_id = job.key.1,
                "Auto-save file download remains pending"
            );
        }
    }
    pub(crate) fn resume_saved_download_admission(&self) -> Result<()> {
        let mut state = self.inner.downloads.0.lock().map_err(|_| Error::Storage)?;
        if state.tasks.iter().any(|task| !task.abort.is_finished())
            || state
                .runner
                .as_ref()
                .is_some_and(|task| !task.abort.is_finished())
        {
            return Err(Error::ManagerStopping);
        }
        state.fenced = false;
        state.tasks.clear();
        state.runner = None;
        Ok(())
    }
    pub(crate) async fn settle_saved_downloads(&self, cancel: bool) -> Result<()> {
        loop {
            let tasks = {
                let mut state = self.inner.downloads.0.lock().map_err(|_| Error::Storage)?;
                if cancel {
                    state.fenced = true;
                    state.queued.clear();
                }
                let mut tasks = state.tasks.clone();
                tasks.extend(state.runner.iter().cloned());
                if cancel {
                    for task in &tasks {
                        task.abort.abort();
                    }
                }
                tasks
            };
            for task in tasks {
                task.wait().await;
            }
            let mut state = self.inner.downloads.0.lock().map_err(|_| Error::Storage)?;
            state.tasks.retain(|task| !task.abort.is_finished());
            if state
                .runner
                .as_ref()
                .is_some_and(|task| task.abort.is_finished())
            {
                state.runner = None;
            }
            if state.tasks.is_empty() && state.runner.is_none() && state.queued.is_empty() {
                return Ok(());
            }
        }
    }
}
