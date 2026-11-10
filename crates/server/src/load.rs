//! Request priorities and load shedding.
//!
//! Every speech request has a priority level, 0..=255, higher more urgent. The worker always
//! serves the most urgent chunk first: by level, then (within a level) a chunk a listener is
//! waiting for before prefetch, then by arrival ([`Priority`]'s order). So the work that can delay
//! a new request's first audio is the work at its level or above.
//!
//! The front estimates that delay. Each live request counts the phoneme ids it has on the worker
//! (waiting and prefetch apart); one that hasn't sent its first chunk yet counts a typical chunk
//! instead. The estimated first-audio latency at level `q` is the time to render
//! - every chunk at a level above `q`,
//! - every waiting chunk at level `q` (prefetch at `q` comes after a new waiting chunk),
//! - and half of the other work the worker may be rendering already, at most one batch of
//!   `max_batch` typical chunks (rendering isn't preempted),
//!
//! at a learned cost per id: an exponentially weighted ratio of the worker's busy time to the ids
//! it rendered, from each Result (busy time since the previous Result, or since the worker was
//! given work while idle). Under load that is throughput, batching included; when the worker
//! is idle it is single-chunk latency.
//!
//! When the estimate at some level is over the budget (`max_estimated_latency_ms`), requests that
//! haven't sent any audio yet are shed, lowest level first and, within a level, the most recently
//! admitted first, until every level's estimate is back under budget. Shedding a request only
//! lowers the estimates at its level and below, so a level is only shed when the work at that
//! level and above is over budget on its own: lower-priority load never sheds higher-priority
//! work (beyond the half chunk it may be rendering). A request that has sent audio is never
//! shed, and nothing of it is cancelled. New requests are admitted the same way: a request whose
//! own admission would leave its level over budget is the newest at its level, so it is the one
//! refused.
//!
//! The count limits (requests in progress, and per API key) still hold. At either, a new request
//! may displace the least urgent request below its level that hasn't sent audio (for a key's
//! limit, one of that key's), which is shed so its permits go to the new one ([`Load::displace`]).
//!
//! The budget applies to the wait from now: a queued request whose remaining wait is under
//! budget keeps its place however long it has waited, since falling back to another provider
//! would take longer still.
//!
//! The only state kept between requests is the learned cost per id and typical chunk size, which
//! are aggregates. Shedding is logged as counts, never with any text.

use crate::protocol::Priority;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// Cost per id before anything is learned: about what the Triton worker takes at batch 1 on the
/// RTX 3060 Ti (30 ms for a typical chunk of 80 ids, `worker/README.md`).
pub const PRIOR_SECS_PER_ID: f64 = 0.000_4;
/// Typical ids per chunk before anything is learned (the median sentence is about 81 phonemes).
pub const PRIOR_CHUNK_IDS: f64 = 80.0;
/// Weight each new sample leaves the history: about the last 20 samples count.
const DECAY: f64 = 0.95;

const QUEUED: u8 = 0;
const STARTED: u8 = 1;
const SHED: u8 = 2;

/// Why a request was refused or stopped before its first audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shed {
    /// The estimated first-audio latency at the request's level when it was shed; zero when it
    /// was shed to free a request permit for a more urgent request.
    pub estimate: Duration,
}

/// The learned cost model: exponentially weighted sums, so their ratio is a weighted average.
#[derive(Debug, Clone)]
struct Cost {
    busy_secs: f64,
    rendered_ids: f64,
    chunk_ids: f64,
}

impl Default for Cost {
    fn default() -> Self {
        let weight = PRIOR_CHUNK_IDS;
        Self { busy_secs: PRIOR_SECS_PER_ID * weight, rendered_ids: weight, chunk_ids: PRIOR_CHUNK_IDS }
    }
}

impl Cost {
    fn secs_per_id(&self) -> f64 {
        self.busy_secs / self.rendered_ids
    }

    fn rendered(&mut self, busy: Duration, ids: u64) {
        self.busy_secs = DECAY * self.busy_secs + busy.as_secs_f64();
        self.rendered_ids = DECAY * self.rendered_ids + ids as f64;
    }

    fn submitted(&mut self, ids: u64) {
        self.chunk_ids = DECAY * self.chunk_ids + (1.0 - DECAY) * ids as f64;
    }
}

/// One admitted request, as the estimate sees it.
struct Tracked {
    level: u8,
    /// The API key it came with, by position in the config.
    key: usize,
    /// Admission order.
    seq: u64,
    state: AtomicU8,
    shed: Notify,
    /// Ids on the worker (submitted, no reply yet), waiting and prefetch.
    waiting_ids: AtomicU64,
    prefetch_ids: AtomicU64,
    /// It has submitted a chunk, so its real work replaces the typical chunk.
    submitted: AtomicBool,
    /// Shed, and a displacement is waiting for the permits it frees.
    claimed: AtomicBool,
    /// The estimate at its level when it was shed, in microseconds.
    shed_estimate_us: AtomicU64,
}

impl Tracked {
    fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    fn counter(&self, waiting: bool) -> &AtomicU64 {
        if waiting { &self.waiting_ids } else { &self.prefetch_ids }
    }

    /// Sheds it unless it has started, noting the `estimate` that shed it. True if it is now shed
    /// by this call.
    fn try_shed(&self, estimate: Duration) -> bool {
        self.shed_estimate_us.store(estimate.as_micros().try_into().unwrap_or(u64::MAX), Ordering::SeqCst);
        let shed = self.state.compare_exchange(QUEUED, SHED, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        if shed {
            self.shed.notify_waiters();
        }
        shed
    }

    fn shed_estimate(&self) -> Duration {
        Duration::from_micros(self.shed_estimate_us.load(Ordering::SeqCst))
    }
}

struct Inner {
    next_seq: u64,
    live: Vec<Arc<Tracked>>,
    cost: Cost,
    log: crate::log::RateLimit,
}

/// The server's load: every live request, and the learned cost. One per server.
pub struct Load {
    /// `None` turns latency-based shedding off.
    budget: Option<Duration>,
    /// The connected worker's largest batch, how much other work may be rendering at once.
    max_batch: Box<dyn Fn() -> u16 + Send + Sync>,
    inner: Mutex<Inner>,
}

impl Load {
    /// `max_batch` reads the connected worker's largest batch. It's called with the load's lock
    /// held, so it mustn't take a lock the worker connection holds while it reports to the load.
    pub fn new(budget: Option<Duration>, max_batch: impl Fn() -> u16 + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            budget,
            max_batch: Box::new(max_batch),
            inner: Mutex::new(Inner {
                next_seq: 0,
                live: Vec::new(),
                cost: Cost::default(),
                log: crate::log::RateLimit::new(Duration::from_secs(10)),
            }),
        })
    }

    /// Admits a request at `level` from API key `key`, shedding queued requests that it puts over
    /// budget; refused if it would itself be over budget.
    pub fn admit(self: &Arc<Self>, level: u8, key: usize) -> Result<Admission, Shed> {
        let mut inner = self.inner.lock().unwrap();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        let tracked = Arc::new(Tracked {
            level,
            key,
            seq,
            state: AtomicU8::new(QUEUED),
            shed: Notify::new(),
            waiting_ids: AtomicU64::new(0),
            prefetch_ids: AtomicU64::new(0),
            submitted: AtomicBool::new(false),
            shed_estimate_us: AtomicU64::new(0),
            claimed: AtomicBool::new(false),
        });
        inner.live.push(tracked.clone());
        self.shed_over_budget(&mut inner);
        if tracked.state() == SHED {
            let estimate = tracked.shed_estimate();
            inner.live.retain(|t| !Arc::ptr_eq(t, &tracked));
            crate::debug!("refused a request at priority {level}: estimated first audio {} ms", estimate.as_millis());
            return Err(Shed { estimate });
        }
        crate::debug!(
            "admitted a request at priority {level}: estimated first audio {} ms",
            self.estimate_locked(&inner, level).as_millis()
        );
        Ok(Admission { load: self.clone(), tracked })
    }

    /// Sheds the least urgent queued request below `level` (lowest level, then newest), from API
    /// key `key` or from any key, to free its permits for a request at `level`. False if there is
    /// none.
    pub fn displace(&self, level: u8, key: Option<usize>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let ours = |t: &Tracked| key.is_none_or(|k| t.key == k);
        // A request already shed is about to free its permits: wait for those rather than shed
        // another, unless another displacement already counts on them.
        if let Some(t) = inner.live.iter().find(|t| ours(t) && t.state() == SHED && !t.claimed.load(Ordering::SeqCst)) {
            t.claimed.store(true, Ordering::SeqCst);
            return true;
        }
        loop {
            let victim = inner
                .live
                .iter()
                .filter(|t| t.level < level && ours(t) && t.state() == QUEUED)
                .min_by_key(|t| (t.level, std::cmp::Reverse(t.seq)))
                .cloned();
            match victim {
                None => return false,
                Some(t) if t.try_shed(Duration::ZERO) => {
                    t.claimed.store(true, Ordering::SeqCst);
                    Self::log_shed(&mut inner, t.level, Duration::ZERO, "to free a request permit");
                    return true;
                }
                // It started meanwhile; look again.
                Some(_) => {}
            }
        }
    }

    /// Whether [`Self::admit`] would refuse a request at `level` now, so a request that would be
    /// refused doesn't displace another first.
    pub fn would_refuse(&self, level: u8) -> bool {
        let Some(budget) = self.budget else { return false };
        let inner = self.inner.lock().unwrap();
        let own = Duration::from_secs_f64(inner.cost.chunk_ids * inner.cost.secs_per_id());
        self.estimate_locked(&inner, level) + own > budget
    }

    /// The estimated time until a new request at `level` would get its first audio.
    pub fn estimate(&self, level: u8) -> Duration {
        self.estimate_locked(&self.inner.lock().unwrap(), level)
    }

    /// The learned render cost per phoneme id.
    pub fn secs_per_id(&self) -> f64 {
        self.inner.lock().unwrap().cost.secs_per_id()
    }

    fn estimate_locked(&self, inner: &Inner, level: u8) -> Duration {
        let mut ids = 0.0;
        // Work that doesn't count here, which the worker may be rendering already.
        let mut behind = 0.0;
        for t in &inner.live {
            let state = t.state();
            let (waiting, prefetch) =
                (t.waiting_ids.load(Ordering::SeqCst) as f64, t.prefetch_ids.load(Ordering::SeqCst) as f64);
            // A shed request's chunks are being cancelled; counting them could shed more.
            if state == SHED {
                continue;
            }
            if t.level < level {
                behind += waiting + prefetch;
                continue;
            }
            ids += waiting;
            if t.level > level {
                ids += prefetch;
            } else {
                behind += prefetch;
            }
            if state == QUEUED && !t.submitted.load(Ordering::SeqCst) {
                ids += inner.cost.chunk_ids;
            }
        }
        // A batch that is already rendering finishes first, whatever its priority: on average,
        // half of it is left. It holds at most `max_batch` typical chunks of that other work.
        ids += behind.min(f64::from((self.max_batch)().max(1)) * inner.cost.chunk_ids) / 2.0;
        Duration::from_secs_f64(ids * inner.cost.secs_per_id())
    }

    /// Sheds queued requests until every level's estimate is under budget. Levels are decided from
    /// the highest down, since shedding at one level only lowers the estimates at that level and
    /// below; within a level the newest goes first.
    fn shed_over_budget(&self, inner: &mut Inner) {
        let Some(budget) = self.budget else { return };
        let mut levels: Vec<u8> = inner.live.iter().filter(|t| t.state() == QUEUED).map(|t| t.level).collect();
        levels.sort_unstable_by(|a, b| b.cmp(a));
        levels.dedup();
        for level in levels {
            loop {
                let estimate = self.estimate_locked(inner, level);
                if estimate <= budget {
                    break;
                }
                let victim = inner
                    .live
                    .iter()
                    .filter(|t| t.level == level && t.state() == QUEUED)
                    .max_by_key(|t| t.seq)
                    .cloned();
                let Some(victim) = victim else { break };
                if victim.try_shed(estimate) {
                    Self::log_shed(inner, level, estimate, "over the latency budget");
                }
            }
        }
    }

    fn log_shed(inner: &mut Inner, level: u8, estimate: Duration, why: &str) {
        if let Some(suppressed) = inner.log.allow() {
            crate::info!(
                "shed a request at priority {level} {why} (estimated first audio {} ms); {suppressed} more shed since the last line",
                estimate.as_millis()
            );
        }
    }

    fn rendered(&self, busy: Duration, ids: u64) {
        self.inner.lock().unwrap().cost.rendered(busy, ids);
    }
}

/// An admitted request. It stays in the estimate until dropped.
pub struct Admission {
    load: Arc<Load>,
    tracked: Arc<Tracked>,
}

impl Admission {
    pub fn level(&self) -> u8 {
        self.tracked.level
    }

    /// Counts a chunk of `ids` ids at this request's level on the worker until the returned
    /// [`Work`] is dropped (the worker's reply arrived, or the connection closed). Adding work can
    /// put queued requests at lower levels over budget, so they may be shed.
    pub fn work(&self, waiting: bool, ids: usize) -> Work {
        let ids = ids as u64;
        let t = &self.tracked;
        t.counter(waiting).fetch_add(ids, Ordering::SeqCst);
        t.submitted.store(true, Ordering::SeqCst);
        let mut inner = self.load.inner.lock().unwrap();
        inner.cost.submitted(ids);
        self.load.shed_over_budget(&mut inner);
        drop(inner);
        Work { owner: Some((self.load.clone(), t.clone())), priority: Priority { level: t.level, waiting }, ids }
    }

    /// Marks the request as started (about to send its first audio). False if it was shed first;
    /// once it returns true, the request is never shed.
    pub fn start(&self) -> bool {
        let t = &self.tracked;
        t.state.compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst).is_ok() || t.state() == STARTED
    }

    /// Resolves once the request is shed.
    pub async fn shed(&self) {
        loop {
            let notified = self.tracked.shed.notified();
            if self.tracked.state() == SHED {
                return;
            }
            notified.await;
        }
    }

    pub fn is_shed(&self) -> bool {
        self.tracked.state() == SHED
    }

    /// The estimate that shed it (zero if it was shed for a request permit, or not shed).
    pub fn shed_estimate(&self) -> Duration {
        self.tracked.shed_estimate()
    }
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let t = &self.tracked;
        f.debug_struct("Admission").field("level", &t.level).field("seq", &t.seq).field("state", &t.state()).finish()
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.load.inner.lock().unwrap().live.retain(|t| !Arc::ptr_eq(t, &self.tracked));
    }
}

/// A chunk on the worker, counted in its request's estimate until dropped.
pub struct Work {
    owner: Option<(Arc<Load>, Arc<Tracked>)>,
    priority: Priority,
    ids: u64,
}

impl Work {
    /// Work outside any request's accounting (tests and tools that talk to the worker directly).
    pub fn untracked(priority: Priority, ids: usize) -> Self {
        Self { owner: None, priority, ids: ids as u64 }
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// The worker rendered it, `busy` after it last finished something or was given work while
    /// idle: a sample for the cost per id.
    pub fn rendered(self, busy: Duration) {
        if let Some((load, _)) = &self.owner {
            load.rendered(busy, self.ids);
        }
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        if let Some((_, t)) = &self.owner {
            t.counter(self.priority.waiting).fetch_sub(self.ids, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU16;

    const MS: Duration = Duration::from_millis(1);

    /// A budget of `chunks` typical chunks at the prior cost, plus a little.
    fn budget(chunks: f64) -> Option<Duration> {
        Some(Duration::from_secs_f64(PRIOR_SECS_PER_ID * PRIOR_CHUNK_IDS * chunks) + MS / 10)
    }

    fn shed_levels(admissions: &[Admission]) -> Vec<(u8, bool)> {
        admissions.iter().map(|a| (a.level(), a.is_shed())).collect()
    }

    #[test]
    fn new_requests_past_the_budget_are_refused() {
        let load = Load::new(budget(3.0), || 1);
        let kept: Vec<_> = (0..3).map(|_| load.admit(10, 0).unwrap()).collect();
        let refused = load.admit(10, 0).unwrap_err();
        assert!(refused.estimate > budget(3.0).unwrap(), "{refused:?}");
        assert!(kept.iter().all(|a| !a.is_shed()));
        // Refused requests leave nothing behind.
        assert_eq!(load.inner.lock().unwrap().live.len(), 3);
    }

    #[test]
    fn higher_levels_shed_the_newest_of_the_lowest_first() {
        let load = Load::new(budget(4.0), || 1);
        let low: Vec<_> = [5, 5, 7, 7].map(|level| load.admit(level, 0).unwrap()).into();
        // Each more urgent request pushes one queued request out, from the bottom up.
        let high = load.admit(200, 0).unwrap();
        assert_eq!(shed_levels(&low), [(5, false), (5, true), (7, false), (7, false)]);
        let high2 = load.admit(200, 0).unwrap();
        assert_eq!(shed_levels(&low), [(5, true), (5, true), (7, false), (7, false)]);
        let high3 = load.admit(100, 0).unwrap();
        assert_eq!(shed_levels(&low), [(5, true), (5, true), (7, false), (7, true)]);
        assert!(!high.is_shed() && !high2.is_shed() && !high3.is_shed());
    }

    #[test]
    fn high_priority_is_shed_only_when_it_alone_is_over_budget() {
        let load = Load::new(budget(2.0), || 1);
        let low = load.admit(1, 0).unwrap();
        let high: Vec<_> = (0..2).map(|_| load.admit(9, 0).unwrap()).collect();
        assert!(low.is_shed());
        assert!(high.iter().all(|a| !a.is_shed()));
        // A third at the top level is over budget on its own, so it is refused, not an older one.
        assert!(load.admit(9, 0).is_err());
        assert!(high.iter().all(|a| !a.is_shed()));
    }

    #[test]
    fn started_requests_are_never_shed_but_still_count() {
        let load = Load::new(budget(2.0), || 1);
        let a = load.admit(1, 0).unwrap();
        let w = a.work(true, PRIOR_CHUNK_IDS as usize);
        assert!(a.start());
        let b = load.admit(1, 0).unwrap();
        // A third: `a` has started, so it can't be shed; `b` and the new one are queued.
        assert!(load.admit(1, 0).is_err());
        let high = load.admit(9, 0).unwrap();
        assert!(!a.is_shed() && b.is_shed() && !high.is_shed());
        drop(w);
        // `a`'s next chunks are counted too; at level 1 its prefetch only adds what may be rendering.
        let _prefetch = a.work(false, PRIOR_CHUNK_IDS as usize);
        assert!(load.estimate(1) < load.estimate(0));
    }

    #[test]
    fn a_request_shed_before_it_starts_cannot_start() {
        let load = Load::new(budget(1.0), || 1);
        let a = load.admit(1, 0).unwrap();
        let _high = load.admit(2, 0).unwrap();
        assert!(a.is_shed());
        assert!(!a.start());
        let b = load.admit(3, 0).unwrap();
        assert!(b.start() && b.start());
    }

    #[test]
    fn displacing_frees_the_least_urgent_queued_request() {
        let load = Load::new(None, || 1);
        let a = load.admit(3, 0).unwrap();
        let b = load.admit(3, 0).unwrap();
        let c = load.admit(1, 0).unwrap();
        let other = load.admit(2, 1).unwrap();
        assert!(c.start());
        assert!(!load.displace(1, None), "nothing queued below level 1");
        assert!(!load.displace(2, Some(0)), "key 0 has nothing queued below level 2");
        assert!(load.displace(4, Some(0)));
        assert_eq!(shed_levels(&[a, b]), [(3, false), (3, true)]);
        assert!(load.displace(4, None));
        assert!(other.is_shed());
    }

    #[test]
    fn displacing_waits_for_a_request_already_shed() {
        let load = Load::new(budget(2.0), || 1);
        let a = load.admit(1, 0).unwrap();
        let b = load.admit(1, 0).unwrap();
        let _high = load.admit(9, 0).unwrap();
        assert!(b.is_shed() && !a.is_shed());
        // `b` is about to free its permits, so the first displacement takes those...
        assert!(load.displace(9, None));
        assert!(!a.is_shed());
        // ...and the next sheds `a`.
        assert!(load.displace(9, None));
        assert!(a.is_shed());
        assert!(!load.displace(9, None));
    }

    #[test]
    fn what_may_be_rendering_counts_up_to_a_batch() {
        let max_batch = Arc::new(AtomicU16::new(1));
        let batch = max_batch.clone();
        let load = Load::new(None, move || batch.load(Ordering::SeqCst));
        let low = load.admit(1, 0).unwrap();
        let high = load.admit(5, 0).unwrap();
        let _w = (low.work(true, 20 * PRIOR_CHUNK_IDS as usize), high.work(true, 1));
        let chunk = load.inner.lock().unwrap().cost.chunk_ids;
        let at_high = || load.estimate(5).as_secs_f64() / load.secs_per_id();
        assert!((at_high() - 1.0 - chunk / 2.0).abs() < 1e-6, "one chunk at batch 1");
        max_batch.store(8, Ordering::SeqCst);
        assert!((at_high() - 1.0 - 4.0 * chunk).abs() < 1e-6, "eight chunks at batch 8");
        max_batch.store(64, Ordering::SeqCst);
        assert!((at_high() - 1.0 - 10.0 * PRIOR_CHUNK_IDS).abs() < 1e-6, "half of it, when that's less");
    }

    #[test]
    fn would_refuse_matches_admission() {
        let load = Load::new(budget(2.0), || 1);
        let _a = load.admit(1, 0).unwrap();
        assert!(!load.would_refuse(1));
        let _b = load.admit(1, 0).unwrap();
        assert!(load.would_refuse(1));
        assert!(load.admit(1, 0).is_err());
        assert!(!load.would_refuse(2));
        assert!(!Load::new(None, || 1).would_refuse(0));
    }

    #[test]
    fn the_cost_per_id_is_learned_from_renders() {
        let load = Load::new(None, || 1);
        let a = load.admit(1, 0).unwrap();
        for _ in 0..200 {
            a.work(true, 100).rendered(Duration::from_millis(10));
        }
        assert!((load.secs_per_id() - 0.000_1).abs() < 1e-6, "{}", load.secs_per_id());
        // Dropped work no longer counts, and the typical chunk is now 100 ids.
        assert!(load.estimate(1).is_zero(), "it has submitted, so no placeholder");
        let _b = load.admit(1, 0).unwrap();
        let estimate = load.estimate(1).as_secs_f64();
        assert!((estimate - 0.01).abs() < 1e-4, "{estimate}");
    }

    #[test]
    fn work_is_counted_by_level_and_kind() {
        let load = Load::new(None, || 1);
        let low = load.admit(1, 0).unwrap();
        let high = load.admit(5, 0).unwrap();
        let work = (low.work(true, 1000), low.work(false, 1000), high.work(true, 10), high.work(false, 20));
        let per = load.secs_per_id();
        let at = |level| load.estimate(level).as_secs_f64() / per;
        // Half a typical chunk for what may be rendering, whenever some work doesn't count.
        let rendering = load.inner.lock().unwrap().cost.chunk_ids / 2.0;
        assert!((at(5) - 10.0 - rendering).abs() < 1e-6, "{}", at(5));
        assert!((at(1) - 1030.0 - rendering).abs() < 1e-6, "low prefetch at its own level");
        assert!((at(0) - 2030.0).abs() < 1e-6, "everything counts");
        assert!((at(6) - rendering).abs() < 1e-6);
        drop(work);
        assert!(at(0).abs() < 1e-6);
    }
}
