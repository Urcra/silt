//! Which model `llama-server` holds and which requests wait (ADR 0072): one
//! model at a time, a swap only once the loaded one's requests are done,
//! and a stop once it has been idle a while. The caller does the loading,
//! unloading and forwarding each step asks for.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::ModelId;

/// A request for a model, from when it's made until it's done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Send the ticket's request on: its model is loaded.
    Run(Ticket),
    /// Its model couldn't be loaded.
    Fail(Ticket),
    /// Stop the server, if one runs, and start it with this model.
    Load(ModelId),
    /// Stop the server.
    Unload,
}

#[derive(Debug, Default)]
pub struct Schedule {
    loaded: Option<ModelId>,
    loading: Option<ModelId>,
    running: usize,
    waiting: VecDeque<(Ticket, ModelId)>,
    idle_since: Option<Instant>,
    next: u64,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn loaded(&self) -> Option<&ModelId> {
        self.loaded.as_ref()
    }

    /// Whether a ticket is waiting behind a swap or a load.
    pub fn is_waiting(&self, ticket: Ticket) -> bool {
        self.waiting.iter().any(|(waiting, _)| *waiting == ticket)
    }

    /// A request for `model`. It runs at once if that model is loaded and no
    /// other model is waited for, so a busy model can't starve a swap.
    pub fn want(&mut self, model: ModelId) -> (Ticket, Vec<Step>) {
        let ticket = Ticket(self.next);
        self.next += 1;
        if self.loading.is_none() && self.waiting.is_empty() && self.loaded.as_ref() == Some(&model) {
            self.running += 1;
            self.idle_since = None;
            return (ticket, vec![Step::Run(ticket)]);
        }
        self.waiting.push_back((ticket, model));
        (ticket, self.next_load())
    }

    /// The server finished loading `model`, or failed to.
    pub fn load_finished(&mut self, model: &ModelId, loaded: bool) -> Vec<Step> {
        if self.loading.as_ref() != Some(model) {
            return Vec::new();
        }
        self.loading = None;
        let mut steps = Vec::new();
        if loaded {
            self.loaded = Some(model.clone());
            while self.waiting.front().is_some_and(|(_, wanted)| wanted == model) {
                let (ticket, _) = self.waiting.pop_front().expect("front");
                self.running += 1;
                steps.push(Step::Run(ticket));
            }
        } else {
            self.loaded = None;
            self.waiting.retain(|(ticket, wanted)| {
                let failed = wanted == model;
                if failed {
                    steps.push(Step::Fail(*ticket));
                }
                !failed
            });
        }
        steps.extend(self.next_load());
        steps
    }

    /// A running request is done, or a waiting one was given up.
    pub fn done(&mut self, ticket: Ticket, now: Instant) -> Vec<Step> {
        if let Some(at) = self.waiting.iter().position(|(waiting, _)| *waiting == ticket) {
            self.waiting.remove(at);
            return self.next_load();
        }
        self.running = self.running.saturating_sub(1);
        if self.running == 0 && self.waiting.is_empty() {
            self.idle_since = Some(now);
        }
        self.next_load()
    }

    /// Unloads a model idle for `idle` or longer.
    pub fn tick(&mut self, now: Instant, idle: Duration) -> Vec<Step> {
        let idle_long = self.idle_since.is_some_and(|since| now.saturating_duration_since(since) >= idle);
        if idle_long && self.running == 0 && self.loading.is_none() && self.waiting.is_empty() && self.loaded.is_some() {
            self.loaded = None;
            self.idle_since = None;
            return vec![Step::Unload];
        }
        Vec::new()
    }

    /// The server stopped: whatever ran there fails, and is done once its
    /// caller hears so.
    pub fn server_exited(&mut self) -> Vec<Step> {
        self.loaded = None;
        self.next_load()
    }

    fn next_load(&mut self) -> Vec<Step> {
        if self.loading.is_some() || self.running > 0 {
            return Vec::new();
        }
        let Some((_, model)) = self.waiting.front() else {
            return Vec::new();
        };
        let model = model.clone();
        if self.loaded.as_ref() == Some(&model) {
            let mut steps = Vec::new();
            while self.waiting.front().is_some_and(|(_, wanted)| *wanted == model) {
                let (ticket, _) = self.waiting.pop_front().expect("front");
                self.running += 1;
                steps.push(Step::Run(ticket));
            }
            return steps;
        }
        self.loaded = None;
        self.idle_since = None;
        self.loading = Some(model.clone());
        vec![Step::Load(model)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> ModelId {
        ModelId::new(name).expect("valid")
    }

    #[test]
    fn the_first_request_loads_and_then_runs() {
        let mut schedule = Schedule::new();
        let (ticket, steps) = schedule.want(id("a"));
        assert_eq!(steps, [Step::Load(id("a"))]);
        assert!(schedule.is_waiting(ticket));
        assert_eq!(schedule.load_finished(&id("a"), true), [Step::Run(ticket)]);
        let (second, steps) = schedule.want(id("a"));
        assert_eq!(steps, [Step::Run(second)]);
    }

    #[test]
    fn a_swap_waits_for_the_loaded_models_requests() {
        let now = Instant::now();
        let mut schedule = Schedule::new();
        let (first, _) = schedule.want(id("a"));
        schedule.load_finished(&id("a"), true);
        let (other, steps) = schedule.want(id("b"));
        assert_eq!(steps, []);
        let (late, steps) = schedule.want(id("a"));
        assert_eq!(steps, [], "a request for the loaded model waits behind the swap");
        assert_eq!(schedule.done(first, now), [Step::Load(id("b"))]);
        assert_eq!(schedule.load_finished(&id("b"), true), [Step::Run(other)]);
        assert_eq!(schedule.done(other, now), [Step::Load(id("a"))]);
        assert_eq!(schedule.load_finished(&id("a"), true), [Step::Run(late)]);
    }

    #[test]
    fn a_failed_load_fails_its_requests_and_moves_on() {
        let mut schedule = Schedule::new();
        let (broken, _) = schedule.want(id("broken"));
        let (good, _) = schedule.want(id("good"));
        assert_eq!(schedule.load_finished(&id("broken"), false), [Step::Fail(broken), Step::Load(id("good"))]);
        assert_eq!(schedule.load_finished(&id("good"), true), [Step::Run(good)]);
    }

    #[test]
    fn an_idle_model_is_unloaded() {
        let start = Instant::now();
        let idle = Duration::from_secs(600);
        let mut schedule = Schedule::new();
        let (ticket, _) = schedule.want(id("a"));
        schedule.load_finished(&id("a"), true);
        assert_eq!(schedule.tick(start + idle * 2, idle), [], "busy");
        schedule.done(ticket, start);
        assert_eq!(schedule.tick(start + idle / 2, idle), []);
        assert_eq!(schedule.tick(start + idle, idle), [Step::Unload]);
        assert_eq!(schedule.loaded(), None);
        let (_, steps) = schedule.want(id("a"));
        assert_eq!(steps, [Step::Load(id("a"))]);
    }

    #[test]
    fn a_request_given_up_while_waiting_leaves_the_queue() {
        let now = Instant::now();
        let mut schedule = Schedule::new();
        let (first, _) = schedule.want(id("a"));
        schedule.load_finished(&id("a"), true);
        let (gone, _) = schedule.want(id("b"));
        assert_eq!(schedule.done(gone, now), []);
        assert!(!schedule.is_waiting(gone));
        assert_eq!(schedule.done(first, now), [], "nothing left to swap to");
    }

    #[test]
    fn a_server_that_exits_is_loaded_again_for_whoever_waits() {
        let mut schedule = Schedule::new();
        let (first, _) = schedule.want(id("a"));
        schedule.load_finished(&id("a"), true);
        let (next, _) = schedule.want(id("b"));
        assert_eq!(schedule.server_exited(), [], "the request that ran there isn't done yet");
        assert_eq!(schedule.done(first, Instant::now()), [Step::Load(id("b"))]);
        assert_eq!(schedule.load_finished(&id("b"), true), [Step::Run(next)]);
    }
}
