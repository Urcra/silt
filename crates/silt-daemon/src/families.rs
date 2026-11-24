//! The families the daemon knows, each held only while in use (ADR 0116):
//! loaded when something reads it, and let go of once unused.
//!
//! What names a family loads it ahead, on the loader's thread, while the
//! core goes on. Anything else that reads one that isn't held loads it
//! there and waits, so no reader ever finds a family missing that is only
//! let go of.

use std::collections::HashMap;
use std::ops::Index;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use silt_git::Repo;
use silt_log::{Delta, Family};
use silt_store::Store;
use silt_types::FamilyId;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::core::CoreError;
use crate::load::{self, Loaded};

/// A family as held: projected, or, without a clone to read git's texts
/// from, its deltas.
pub(crate) enum Held {
    /// With how much of its log its stored snapshot covers, and whether
    /// that snapshot is in an older format, to take again.
    Projected {
        family: Box<Family>,
        covered: usize,
        outdated: bool,
    },
    Raw(Vec<Delta>),
}

impl From<Loaded> for Held {
    fn from(loaded: Loaded) -> Self {
        match loaded {
            Loaded::Projected { family, covered, outdated } => Self::Projected { family, covered, outdated },
            Loaded::Raw(deltas) => Self::Raw(deltas),
        }
    }
}

struct Slot {
    held: OnceLock<Held>,
    /// Whether it was projected when last held, or is to be once loaded.
    projected: bool,
    /// Where its clone is, to load it from; none for a family without one.
    root: Option<PathBuf>,
    /// When something last used it, in milliseconds since `started`.
    used: AtomicU64,
    /// About how much it takes held, as the store holds it; [`UNSIZED`]
    /// until known. The loader reads it with the family, so a load from a
    /// shared reference records it too.
    size: AtomicU64,
}

const UNSIZED: u64 = u64::MAX;

pub(crate) struct Families {
    slots: HashMap<FamilyId, Slot>,
    loader: Option<Loader>,
    started: Instant,
}

impl Families {
    /// With no loader, a family let go of can't be loaded again: a hook's
    /// core holds what it loaded until it ends.
    pub(crate) fn new(loader: Option<Loader>) -> Self {
        Self { slots: HashMap::new(), loader, started: Instant::now() }
    }

    /// Notes a family not held yet, to load from `root` when used.
    pub(crate) fn know(&mut self, family: FamilyId, root: Option<PathBuf>) {
        let slot = self.slot(family);
        slot.projected = root.is_some();
        slot.root = root;
    }

    /// Holds what was loaded of `family` from the clone at `root`.
    pub(crate) fn hold(&mut self, family: FamilyId, loaded: Loaded, root: Option<PathBuf>) {
        let held = Held::from(loaded);
        let used = self.now();
        let slot = self.slot(family);
        slot.projected = matches!(held, Held::Projected { .. });
        if root.is_some() {
            slot.root = root;
        }
        slot.held = OnceLock::from(held);
        *slot.used.get_mut() = used;
    }

    /// Holds a family projected here, its snapshot covering none of it.
    pub(crate) fn insert(&mut self, family: FamilyId, projected: Family) {
        let loaded = Loaded::Projected { family: Box::new(projected), covered: 0, outdated: false };
        self.hold(family, loaded, None);
    }

    /// Forgets the family, as erased or replaced, held or not.
    pub(crate) fn remove(&mut self, family: &FamilyId) -> Option<Held> {
        self.slots.remove(family).and_then(|slot| slot.held.into_inner())
    }

    /// Lets go of the family, to load again when used.
    pub(crate) fn unload(&mut self, family: FamilyId) -> Option<Held> {
        self.slots.get_mut(&family).and_then(|slot| slot.held.take())
    }

    /// The projected family, loaded if it isn't held.
    pub(crate) fn get(&self, family: &FamilyId) -> Option<&Family> {
        match self.held(family)? {
            Held::Projected { family, .. } => Some(family),
            Held::Raw(_) => None,
        }
    }

    pub(crate) fn get_mut(&mut self, family: &FamilyId) -> Option<&mut Family> {
        match self.held_mut(family)? {
            Held::Projected { family, .. } => Some(family),
            Held::Raw(_) => None,
        }
    }

    /// The projected family if it is held, without loading it or counting
    /// as a use: for what reads every family.
    pub(crate) fn loaded(&self, family: &FamilyId) -> Option<&Family> {
        match self.slots.get(family)?.held.get()? {
            Held::Projected { family, .. } => Some(family),
            Held::Raw(_) => None,
        }
    }

    pub(crate) fn loaded_mut(&mut self, family: &FamilyId) -> Option<&mut Family> {
        match self.slots.get_mut(family)?.held.get_mut()? {
            Held::Projected { family, .. } => Some(family),
            Held::Raw(_) => None,
        }
    }

    /// The deltas of a family without a clone here if they are held,
    /// without loading them or counting as a use.
    pub(crate) fn loaded_raw(&self, family: &FamilyId) -> Option<&Vec<Delta>> {
        match self.slots.get(family)?.held.get()? {
            Held::Raw(deltas) => Some(deltas),
            Held::Projected { .. } => None,
        }
    }

    /// The deltas of a family without a clone here, loaded if not held.
    pub(crate) fn raw(&self, family: &FamilyId) -> Option<&Vec<Delta>> {
        match self.held(family)? {
            Held::Raw(deltas) => Some(deltas),
            Held::Projected { .. } => None,
        }
    }

    pub(crate) fn raw_mut(&mut self, family: &FamilyId) -> Option<&mut Vec<Delta>> {
        match self.held_mut(family)? {
            Held::Raw(deltas) => Some(deltas),
            Held::Projected { .. } => None,
        }
    }

    /// The deltas of a family without a clone here, none yet if it is new.
    pub(crate) fn raw_entry(&mut self, family: FamilyId) -> &mut Vec<Delta> {
        if !self.knows(&family) {
            self.hold(family, Loaded::Raw(Vec::new()), None);
        }
        self.raw_mut(&family).expect("a family without a clone")
    }

    /// Whether the family is projected here, held or not.
    pub(crate) fn contains_key(&self, family: &FamilyId) -> bool {
        self.slots.get(family).is_some_and(|slot| slot.projected)
    }

    /// Whether the family is here at all, projected or not, held or not.
    pub(crate) fn knows(&self, family: &FamilyId) -> bool {
        self.slots.contains_key(family)
    }

    pub(crate) fn is_loaded(&self, family: &FamilyId) -> bool {
        self.slots.get(family).is_some_and(|slot| slot.held.get().is_some())
    }

    /// Every family here, projected or not.
    pub(crate) fn ids(&self) -> Vec<FamilyId> {
        let mut ids: Vec<FamilyId> = self.slots.keys().copied().collect();
        ids.sort();
        ids
    }

    /// The projected families, held or not.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &FamilyId> {
        self.slots.iter().filter(|(_, slot)| slot.projected).map(|(id, _)| id)
    }

    /// The families held now.
    pub(crate) fn loaded_ids(&self) -> Vec<FamilyId> {
        let mut ids: Vec<FamilyId> = self.slots.iter().filter(|(_, slot)| slot.held.get().is_some()).map(|(id, _)| *id).collect();
        ids.sort();
        ids
    }

    /// The projected families held now.
    pub(crate) fn iter_loaded(&self) -> impl Iterator<Item = (&FamilyId, &Family)> {
        self.slots.iter().filter_map(|(id, slot)| match slot.held.get()? {
            Held::Projected { family, .. } => Some((id, family.as_ref())),
            Held::Raw(_) => None,
        })
    }

    pub(crate) fn loaded_values_mut(&mut self) -> impl Iterator<Item = &mut Family> {
        self.slots.values_mut().filter_map(|slot| match slot.held.get_mut()? {
            Held::Projected { family, .. } => Some(family.as_mut()),
            Held::Raw(_) => None,
        })
    }

    /// How much of the family's log its stored snapshot covers, and whether
    /// that snapshot is in an older format.
    pub(crate) fn snapshotted(&self, family: &FamilyId) -> Option<(usize, bool)> {
        match self.slots.get(family)?.held.get()? {
            Held::Projected { covered, outdated, .. } => Some((*covered, *outdated)),
            Held::Raw(_) => None,
        }
    }

    /// Notes a snapshot covering `covers` of the family's log was stored.
    pub(crate) fn set_snapshotted(&mut self, family: &FamilyId, covers: usize) {
        if let Some(Held::Projected { covered, outdated, .. }) = self.slots.get_mut(family).and_then(|slot| slot.held.get_mut()) {
            *covered = covers;
            *outdated = false;
        }
    }

    /// How long since something last used the family.
    pub(crate) fn unused_for(&self, family: &FamilyId) -> Duration {
        let used = self.slots.get(family).map_or(0, |slot| slot.used.load(Ordering::Relaxed));
        Duration::from_millis(self.now().saturating_sub(used))
    }

    pub(crate) fn size(&self, family: &FamilyId) -> Option<u64> {
        Some(self.slots.get(family)?.size.load(Ordering::Relaxed)).filter(|size| *size != UNSIZED)
    }

    pub(crate) fn set_size(&mut self, family: &FamilyId, size: u64) {
        if let Some(slot) = self.slots.get_mut(family) {
            *slot.size.get_mut() = size.min(UNSIZED - 1);
        }
    }

    /// Where the family's clone is, as last known.
    pub(crate) fn root(&self, family: &FamilyId) -> Option<&Path> {
        self.slots.get(family)?.root.as_deref()
    }

    pub(crate) fn loader(&self) -> Option<&Loader> {
        self.loader.as_ref()
    }

    fn held(&self, family: &FamilyId) -> Option<&Held> {
        let slot = self.slots.get(family)?;
        slot.used.store(self.now(), Ordering::Relaxed);
        if let Some(held) = slot.held.get() {
            return Some(held);
        }
        let Fetched { result, size } = self.load_now(*family, slot.root.clone())?;
        if let Some(size) = size {
            slot.size.store(size.min(UNSIZED - 1), Ordering::Relaxed);
        }
        Some(slot.held.get_or_init(|| result.into()))
    }

    fn held_mut(&mut self, family: &FamilyId) -> Option<&mut Held> {
        self.held(family)?;
        self.slots.get_mut(family)?.held.get_mut()
    }

    fn load_now(&self, family: FamilyId, root: Option<PathBuf>) -> Option<Fetched<Loaded>> {
        let loader = self.loader.as_ref()?;
        match loader.load_blocking(family, root) {
            Ok(Fetched { result: Ok(loaded), size }) => Some(Fetched { result: loaded, size }),
            Ok(Fetched { result: Err(error), .. }) | Err(error) => {
                warn!(%error, %family, "could not load the family");
                None
            }
        }
    }

    fn slot(&mut self, family: FamilyId) -> &mut Slot {
        let used = self.now();
        self.slots.entry(family).or_insert_with(|| Slot {
            held: OnceLock::new(),
            projected: false,
            root: None,
            used: AtomicU64::new(used),
            size: AtomicU64::new(UNSIZED),
        })
    }

    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

impl Index<&FamilyId> for Families {
    type Output = Family;

    fn index(&self, family: &FamilyId) -> &Family {
        self.get(family).expect("the family is projected here")
    }
}

/// Loads families on a thread of its own, from a store connection of its
/// own, so a reader can wait for one without the core's runtime.
#[derive(Debug, Clone)]
pub(crate) struct Loader {
    jobs: mpsc::UnboundedSender<Job>,
}

#[derive(Debug)]
struct Job {
    family: FamilyId,
    root: Option<PathBuf>,
    reply: Reply,
}

#[derive(Debug)]
enum Reply {
    Waiting(std::sync::mpsc::SyncSender<Fetched>),
    Later(oneshot::Sender<Fetched>),
}

/// What the loader loaded of a family, with how much the store holds of it,
/// read with it so the core's tick needn't ask.
#[derive(Debug)]
pub(crate) struct Fetched<T = Result<Loaded, CoreError>> {
    pub(crate) result: T,
    pub(crate) size: Option<u64>,
}

impl Loader {
    pub(crate) fn start(data_dir: &Path) -> std::io::Result<Self> {
        let (jobs, mut queued) = mpsc::unbounded_channel::<Job>();
        let dir = data_dir.to_owned();
        std::thread::Builder::new().name("silt-loader".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    warn!(%error, "families can't be loaded on demand");
                    return;
                }
            };
            runtime.block_on(async move {
                let store = Store::open(&dir).await;
                while let Some(job) = queued.recv().await {
                    let loaded = match &store {
                        Ok(store) => {
                            let repo = job.root.as_deref().and_then(|root| Repo::discover(root).ok());
                            load::family_unread(store, job.family, repo.as_ref()).await
                        }
                        Err(error) => Err(CoreError::Invalid(format!("the loader's store didn't open: {error}"))),
                    };
                    let size = match (&store, &loaded) {
                        (Ok(store), Ok(_)) => store.family_size(job.family).await.ok(),
                        _ => None,
                    };
                    let fetched = Fetched { result: loaded, size };
                    match job.reply {
                        Reply::Waiting(reply) => {
                            let _ = reply.send(fetched);
                        }
                        Reply::Later(reply) => {
                            let _ = reply.send(fetched);
                        }
                    }
                }
            });
        })?;
        Ok(Self { jobs })
    }

    /// Loads the family, waiting for it.
    fn load_blocking(&self, family: FamilyId, root: Option<PathBuf>) -> Result<Fetched, CoreError> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.jobs
            .send(Job { family, root, reply: Reply::Waiting(reply) })
            .map_err(|_| CoreError::Invalid("the loader stopped".into()))?;
        answer.recv().map_err(|_| CoreError::Invalid("the loader stopped".into()))
    }

    /// Loads the family, answering when done.
    pub(crate) fn load_later(&self, family: FamilyId, root: Option<PathBuf>) -> oneshot::Receiver<Fetched> {
        let (reply, answer) = oneshot::channel();
        let _ = self.jobs.send(Job { family, root, reply: Reply::Later(reply) });
        answer
    }
}
