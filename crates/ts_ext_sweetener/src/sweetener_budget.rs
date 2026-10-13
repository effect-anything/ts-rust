//! Resource budgets for one expansion
//!
//! Upstream: `shared/src/limits.ts` (`ResourceBudget`). Each limit has the
//! upstream default. Going past one is `SWR4015` at the invocation being
//! expanded; after that nothing else in the file is expanded.

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Limits for one file's expansion. `Default` is upstream's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_expansion_steps: u64,
    pub max_template_steps: u64,
    pub max_matcher_steps: u64,
    /// Upstream's `maxNestingDepth`. Re-expansion is still bounded by the
    /// printer's own, smaller depth limit.
    pub max_nesting_depth: u64,
    pub deadline: Option<Duration>,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_input_tokens: 1_000_000,
            max_output_tokens: 4_000_000,
            max_expansion_steps: 1_000_000,
            max_template_steps: 1_000_000,
            max_matcher_steps: 10_000_000,
            max_nesting_depth: 1024,
            deadline: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    InputTokens,
    OutputTokens,
    ExpansionSteps,
    TemplateSteps,
    MatcherSteps,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::InputTokens => "input-tokens",
            Kind::OutputTokens => "output-tokens",
            Kind::ExpansionSteps => "expansion-steps",
            Kind::TemplateSteps => "template-steps",
            Kind::MatcherSteps => "matcher-steps",
        }
    }
}

struct Tracker {
    budget: ResourceBudget,
    started: Instant,
    used: [u64; 5],
    /// The macro being expanded, for the message.
    current: String,
    /// The `SWR4015` message once a limit is passed.
    exhausted: Option<String>,
}

thread_local! {
    static TRACKER: RefCell<Option<Tracker>> = const { RefCell::new(None) };
    static CONFIGURED: RefCell<ResourceBudget> = RefCell::new(ResourceBudget::default());
}

/// Run `f` with `budget` as the limits of every pass it starts.
pub(crate) fn with_budget<T>(budget: ResourceBudget, f: impl FnOnce() -> T) -> T {
    let previous = CONFIGURED.with(|c| std::mem::replace(&mut *c.borrow_mut(), budget));
    let out = f();
    CONFIGURED.with(|c| *c.borrow_mut() = previous);
    out
}

/// The limits the next pass on this thread will use.
pub(crate) fn configured() -> ResourceBudget {
    CONFIGURED.with(|c| c.borrow().clone())
}

/// The nesting limit of the running pass.
pub(crate) fn max_nesting_depth() -> u64 {
    TRACKER.with(|t| {
        t.borrow()
            .as_ref()
            .map_or(ResourceBudget::default().max_nesting_depth, |t| {
                t.budget.max_nesting_depth
            })
    })
}

/// Stack for an expansion thread: a base plus a frame budget per nesting
/// level. Only touched pages are committed, so reserving is cheap.
pub(crate) fn stack_bytes(budget: &ResourceBudget) -> usize {
    const BASE: usize = 16 << 20;
    const PER_LEVEL: usize = 512 << 10;
    let levels = usize::try_from(budget.max_nesting_depth.min(1 << 16)).unwrap_or(1 << 16);
    BASE.saturating_add(levels.saturating_mul(PER_LEVEL))
}

/// Start a pass: fresh counters, the deadline clock restarted.
pub(crate) fn reset() {
    let budget = CONFIGURED.with(|c| c.borrow().clone());
    TRACKER.with(|t| {
        *t.borrow_mut() = Some(Tracker {
            budget,
            started: Instant::now(),
            used: [0; 5],
            current: String::new(),
            exhausted: None,
        });
    });
}

/// The macro the next charges belong to.
pub(crate) fn enter(name: &str) {
    TRACKER.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut()
            && t.exhausted.is_none()
        {
            t.current = name.to_owned();
        }
    });
}

pub(crate) fn exhausted() -> bool {
    TRACKER.with(|t| t.borrow().as_ref().is_some_and(|t| t.exhausted.is_some()))
}

/// Charge `amount` of `kind`. `Err` is the `SWR4015` message the first
/// time a limit (or the deadline) is passed; afterwards every charge fails
/// with the same message so each later invocation stays as written.
pub(crate) fn charge(kind: Kind, amount: u64) -> Result<(), String> {
    TRACKER.with(|t| {
        let mut t = t.borrow_mut();
        let Some(t) = t.as_mut() else {
            return Ok(());
        };
        let message = |t: &Tracker, label: &str| {
            let name = if t.current.is_empty() {
                "this file"
            } else {
                t.current.as_str()
            };
            format!(
                "SWR4015: Expanding {name} reached the {label} limit. No further macros in it were expanded."
            )
        };
        if let Some(message) = &t.exhausted {
            return Err(message.clone());
        }
        let slot = &mut t.used[kind as usize];
        *slot = slot.saturating_add(amount);
        let limit = match kind {
            Kind::InputTokens => t.budget.max_input_tokens,
            Kind::OutputTokens => t.budget.max_output_tokens,
            Kind::ExpansionSteps => t.budget.max_expansion_steps,
            Kind::TemplateSteps => t.budget.max_template_steps,
            Kind::MatcherSteps => t.budget.max_matcher_steps,
        };
        let label = if *slot > limit {
            kind.label()
        } else if t.budget.deadline.is_some_and(|d| t.started.elapsed() > d) {
            "deadline"
        } else {
            return Ok(());
        };
        let message = message(t, label);
        t.exhausted = Some(message.clone());
        Err(message)
    })
}

/// Tokens in `text`, the way upstream counts them closely enough for a
/// limit: identifiers, numbers and strings are one each, every other
/// non-space character is one.
pub(crate) fn count_tokens(text: &str) -> u64 {
    let mut count = 0u64;
    let mut in_word = false;
    for ch in text.chars() {
        let word = ch == '_' || ch == '$' || ch.is_alphanumeric();
        if word {
            if !in_word {
                count += 1;
            }
        } else if !ch.is_whitespace() {
            count += 1;
        }
        in_word = word;
    }
    count
}
