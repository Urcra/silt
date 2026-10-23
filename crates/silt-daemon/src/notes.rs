//! Git notes on the commits threads record (ADR 0042): written in the
//! checkout as each commit is recorded, merged into the clone it came from,
//! and pushed to origin when the project says so.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use silt_log::{Note, NoteFile, Origins};
use silt_proto::GitNotes;
use silt_types::{DeltaId, FamilyId, Frontier, GitOid, ThreadId};
use tracing::warn;

use crate::core::{Core, Event};

/// How often each server's project settings are read again.
const SETTINGS_INTERVAL: Duration = Duration::from_secs(30);

/// What each server says of its projects' notes, by server and repository.
#[derive(Debug, Default)]
pub(crate) struct NotesSettings {
    by_project: HashMap<(String, String), GitNotes>,
    asked: HashMap<String, Instant>,
    /// The servers whose settings were read since the daemon started.
    read: HashSet<String>,
    /// Commits recorded before their server's settings were read.
    waiting: Vec<Waiting>,
}

#[derive(Debug)]
struct Waiting {
    server: String,
    family: FamilyId,
    thread: ThreadId,
    checkout: PathBuf,
    commit: GitOid,
    recorded: DeltaId,
}

impl Core {
    /// Asks each server for its projects' settings once they're due.
    pub(crate) fn drive_notes_settings(&mut self) {
        for remote in self.remotes.clone() {
            let server = remote.config.url.clone();
            if self.notes_settings.asked.get(&server).is_some_and(|asked| asked.elapsed() < SETTINGS_INTERVAL) {
                continue;
            }
            self.notes_settings.asked.insert(server.clone(), Instant::now());
            let events = self.events.clone();
            tokio::spawn(async move {
                match remote.projects().await {
                    Ok(reply) => {
                        let projects = reply.projects.into_iter().map(|project| (project.repo, project.git_notes)).collect();
                        let _ = events.send(Event::NotesSettings { server, projects });
                    }
                    Err(error) => warn!(%error, server, "could not read the server's project settings"),
                }
            });
        }
    }

    pub(crate) fn notes_settings_read(&mut self, server: &str, projects: Vec<(String, GitNotes)>) {
        self.notes_settings.by_project.retain(|(known, _), _| known != server);
        for (repo, notes) in projects {
            self.notes_settings.by_project.insert((server.to_owned(), repo), notes);
        }
        self.notes_settings.read.insert(server.to_owned());
        let (ready, waiting) =
            std::mem::take(&mut self.notes_settings.waiting).into_iter().partition(|waiting| waiting.server == server);
        self.notes_settings.waiting = waiting;
        for Waiting { family, thread, checkout, commit, recorded, .. } in ready {
            self.note_commit(family, thread, checkout, commit, recorded);
        }
    }

    /// Reads every server's settings now, for a round without the daemon.
    pub(crate) async fn read_notes_settings(&mut self) {
        for remote in self.remotes.clone() {
            match remote.projects().await {
                Ok(reply) => {
                    let projects = reply.projects.into_iter().map(|project| (project.repo, project.git_notes)).collect();
                    self.notes_settings_read(&remote.config.url, projects);
                }
                Err(error) => {
                    warn!(%error, server = remote.config.url, "could not read the server's project settings");
                    // The clone's own setting is all this round has.
                    self.notes_settings.read.insert(remote.config.url.clone());
                }
            }
        }
    }

    /// The thread's project's setting: its server's, else the clone's own.
    fn notes_for(&self, thread: ThreadId, repo: &silt_git::Repo) -> GitNotes {
        let project = self.threads.get(&thread).map(|record| record.repo.clone()).unwrap_or_default();
        let server = self.remote_for(thread).map(|remote| remote.config.url.clone());
        if let Some(notes) = server.and_then(|server| self.notes_settings.by_project.get(&(server, project))) {
            return *notes;
        }
        repo.notes_config().and_then(|value| value.parse().ok()).unwrap_or_default()
    }

    /// Notes `commit`, which `recorded` just recorded on `thread`, when the
    /// project wants notes, then shares them in the background.
    pub(crate) fn note_commit(
        &mut self,
        family: FamilyId,
        thread: ThreadId,
        checkout: PathBuf,
        commit: GitOid,
        recorded: DeltaId,
    ) {
        if let Some(server) = self.remote_for(thread).map(|remote| remote.config.url.clone())
            && !self.notes_settings.read.contains(&server)
        {
            self.notes_settings.waiting.push(Waiting { server, family, thread, checkout, commit, recorded });
            return;
        }
        let Ok(repo) = silt_git::Repo::discover(&checkout) else {
            return;
        };
        let setting = self.notes_for(thread, &repo);
        if setting == GitNotes::Off {
            return;
        }
        let Some(note) = self.note_of(family, thread, &repo, commit, recorded) else {
            return;
        };
        if let Err(error) = repo.add_note(commit, &note.render()) {
            warn!(%error, %commit, "could not write the commit's note");
            return;
        }
        tokio::task::spawn_blocking(move || share_notes(&checkout, setting));
    }

    fn note_of(
        &self,
        family_id: FamilyId,
        thread: ThreadId,
        repo: &silt_git::Repo,
        commit: GitOid,
        recorded: DeltaId,
    ) -> Option<Note> {
        let family = self.families.get(&family_id)?;
        let changed = match repo.changed_lines(commit) {
            Ok(changed) => changed,
            Err(error) => {
                warn!(%error, %commit, "could not read what the commit changed");
                return None;
            }
        };
        let at = family.graph().version_vector(&Frontier::single(recorded));
        let origins = Origins::new(family.log());
        let mut files = Vec::new();
        let mut turns = BTreeSet::new();
        for (path, lines) in changed {
            let (Some(committed), Some(ours)) = (repo.text_at(commit, &path), family.lines_at(&path, &at)) else {
                continue;
            };
            let ours: Vec<_> = ours.into_iter().map(|(text, id)| (text, id.and_then(|id| origins.of(id)))).collect();
            let runs = silt_log::runs_of(&committed, &lines, &ours, thread);
            turns.extend(runs.iter().filter(|run| run.thread.is_none()).filter_map(|run| run.turn));
            if !runs.is_empty() {
                files.push(NoteFile { path, runs });
            }
        }
        let known = self.turns(thread);
        let prompt = |turn: usize| known.get(turn - 1).map(|info| info.prompt.clone()).unwrap_or_default();
        let title = family
            .thread(thread)
            .and_then(|state| state.title().map(str::to_owned))
            .or_else(|| known.first().map(|turn| turn.prompt.clone()));
        Some(Note {
            thread,
            link: self.remote_for(thread).map(|remote| format!("{}/app/t/{thread}", remote.config.url.trim_end_matches('/'))),
            title: title.filter(|title| !title.trim().is_empty()),
            turns: turns.into_iter().map(|turn| (turn, prompt(turn))).collect(),
            files,
        })
    }
}

/// Merges the checkout's notes into the clone it came from and, with
/// `push`, into origin's. A failure is tried again with the next commit.
fn share_notes(checkout: &std::path::Path, setting: GitNotes) {
    let Ok(repo) = silt_git::Repo::discover(checkout) else {
        return;
    };
    // Origin first, so the clone gets what other machines noted too.
    let mut remotes = Vec::new();
    if setting == GitNotes::Push && repo.has_remote("origin") {
        remotes.push("origin");
    }
    if repo.has_remote("local") {
        remotes.push("local");
    }
    for remote in remotes {
        if let Err(error) = repo.sync_notes(remote) {
            warn!(%error, remote, "could not share the commits' notes");
        }
    }
}
