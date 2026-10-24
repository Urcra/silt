//! Claude Code cloud sessions started and steered from the web UI, through
//! the person's own `claude` (ADR 0040).

use std::path::Path;

use silt_agents::cloud;
use silt_types::CloudSessionId;
use tokio::sync::oneshot;

use crate::core::{Core, CoreError, Event, now_ms};
use crate::protocol::{BusEvent, CloudLaunch, Request};

impl Core {
    /// Starts a cloud session for a clone this machine knows. The launch is
    /// kept once claude names the session, until its thread arrives.
    pub(crate) fn cloud_start(
        &self,
        clone: &Path,
        task: &str,
    ) -> Result<impl Future<Output = Result<CloudLaunch, CoreError>> + Send + 'static, CoreError> {
        let clone = silt_os::canonicalize(clone).map_err(|_| CoreError::Invalid(format!("no clone at {}", clone.display())))?;
        let Some(repo) = self
            .clones
            .iter()
            .find(|(_, path)| silt_os::canonicalize(path).is_ok_and(|path| path == clone))
            .map(|(repo, _)| repo)
        else {
            return Err(CoreError::Invalid(format!("{} isn't a project's clone on this machine", clone.display())));
        };
        let (repo, task, events) = (repo.clone(), task.to_owned(), self.events.clone());
        Ok(async move {
            let started = {
                let task = task.clone();
                tokio::task::spawn_blocking(move || cloud::start(&clone, &task, cloud::START_TIMEOUT))
                    .await
                    .map_err(|error| CoreError::Invalid(error.to_string()))?
                    .map_err(|error| CoreError::Invalid(error.to_string()))?
            };
            let launch = CloudLaunch {
                id: started.id.to_string(),
                url: started.id.url(),
                repo,
                task,
                started_ms: now_ms(),
                warnings: started.warnings,
                thread: None,
            };
            let (reply, answer) = oneshot::channel();
            if events.send(Event::Request(Request::CloudLaunched { launch: launch.clone() }, reply)).is_ok() {
                let _ = answer.await;
            }
            Ok(launch)
        })
    }

    pub(crate) fn cloud_launched(&mut self, launch: CloudLaunch) {
        self.cloud_launches.push(launch);
        self.announce(BusEvent::Threads);
    }

    /// The launches, each with the thread recording it once one does.
    pub(crate) fn cloud_launches(&self) -> Vec<CloudLaunch> {
        let recorded: Vec<(CloudSessionId, String)> = self
            .threads
            .values()
            .filter_map(|record| {
                let state = self.families.get(&record.family)?.thread(record.id)?;
                Some((state.cloud_session()?.clone(), record.id.to_string()))
            })
            .collect();
        self.cloud_launches
            .iter()
            .map(|launch| {
                let id = launch.id.parse::<CloudSessionId>().ok();
                let thread =
                    recorded.iter().find(|(recorded, _)| Some(recorded) == id.as_ref()).map(|(_, thread)| thread.clone());
                CloudLaunch { thread, ..launch.clone() }
            })
            .collect()
    }

    /// Queues `text` into the cloud session the thread records.
    pub(crate) fn cloud_send(
        &self,
        thread: &str,
        text: &str,
    ) -> Result<impl Future<Output = Result<(), CoreError>> + Send + 'static, CoreError> {
        let thread = self.find_thread(thread)?;
        let family = self.threads.get(&thread).map(|record| record.family);
        let id = family
            .and_then(|family| self.families.get(&family)?.thread(thread)?.cloud_session().cloned())
            .ok_or_else(|| CoreError::Invalid("the thread records no cloud session".into()))?;
        if text.trim().is_empty() {
            return Err(CoreError::Invalid("the message is empty".into()));
        }
        let text = text.to_owned();
        Ok(async move {
            tokio::task::spawn_blocking(move || cloud::send(&id, &text))
                .await
                .map_err(|error| CoreError::Invalid(error.to_string()))?
                .map_err(|error| CoreError::Invalid(error.to_string()))
        })
    }
}
