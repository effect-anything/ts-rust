//! Incremental expansion: the file's top-level items, replayed.
//!
//! Each pass over a file prints its top-level items one at a time
//! (`Printer::print_step`). The pass keeps a log of every item it printed:
//! the text and maps it appended, and the state the printer carries to the
//! next item. The next expansion of the same file (an editor edit) replays
//! an item from that log instead of printing it again when nothing the item
//! reads changed:
//!
//! - the pass inputs (`Recorder::fingerprint`): the macros, classes and
//!   runtime imports, the file's syntax imports and parameters, the reader's
//!   operators, the filename, the phase and the pass;
//! - the text around the item (`window`): from where the previous item
//!   stopped copying, and a few nodes before it, to well past the nodes it
//!   took. Printing an item looks at its neighbours, never much further;
//! - what an item reads of the whole file, made explicit here: the open
//!   JSX tags before it, the `for syntax` imports before it, the local macro
//!   definitions after it, the context regions in its window;
//! - the state the printer carries between items (`State`), hashed;
//! - the user names it asked about (`UserNames`), with the same answers;
//! - the resource budget: the item's charges still fit.
//!
//! An item whose incoming or outgoing state holds an expansion in progress
//! (a pending trace, an exhausted budget, a placeholder another item could
//! refer to) is printed and not logged. The positions an item recorded are
//! moved by the edit (one changed range between the two texts); an item
//! whose window or positions touch the changed range is printed again.
//! Placeholder ids in the replayed text get fresh ids, as a fresh print
//! would. The final hygiene and hoist passes still run over the whole
//! output, as before.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use super::{
    ACTIVE_EXPANSIONS, BUDGET, CURRENT_INVOCATION, Diagnostic, IMPORT_OWNERS, INVOKED_OWNERS,
    LITERAL_SINK, LOCAL_HEADS, OWNER_FILTER, Printer, SYNTAX_PARAMETERS, Segment, SegmentKind,
    Syntax, TRACES, UNKNOWN_PARAMETERIZED,
};
use crate::sweetener_cache::Probe;
use crate::sweetener_context::Contexts;
use crate::sweetener_scoped::Unused;

/// Nodes before an item that its printing may look at.
const LOOK_BEHIND: usize = 8;
/// Nodes after the ones an item took that its printing may look at. The
/// infix walk stops 40 nodes on.
const LOOK_AHEAD: usize = 64;

/// The names the file binds, as the printer asks about them. A logged item
/// records each question and its answer, so it is replayed only where every
/// answer is the same.
#[derive(Clone, Debug, Default)]
pub(super) struct UserNames {
    set: std::collections::HashSet<String>,
    /// Names inserted after the pass started, in order (with repeats).
    added: Vec<String>,
}

thread_local! {
    static NAME_LOG: std::cell::RefCell<Option<Vec<(String, bool)>>> =
        const { std::cell::RefCell::new(None) };
}

impl UserNames {
    pub(super) fn new(set: std::collections::HashSet<String>) -> Self {
        Self {
            set,
            added: Vec::new(),
        }
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        let found = self.set.contains(name);
        NAME_LOG.with(|log| {
            if let Some(log) = log.borrow_mut().as_mut() {
                log.push((name.to_owned(), found));
            }
        });
        found
    }

    /// Every insert is logged, a name already there too: a replayed item
    /// inserts it again where an earlier item no longer did.
    pub(super) fn insert(&mut self, name: String) {
        self.set.insert(name.clone());
        self.added.push(name);
    }

    fn answers(&self, log: &[(String, bool)]) -> bool {
        log.iter()
            .all(|(name, found)| self.set.contains(name) == *found)
    }
}

/// The state the printer carries from one item to the next, at an item
/// boundary.
#[derive(Clone, Debug)]
struct State {
    last_original: usize,
    hygiene: Vec<(String, String)>,
    imports_emitted: bool,
    form_signals: Vec<(String, String)>,
    last_form_next: usize,
    channel_renames: Vec<(String, String)>,
    context_rejected: String,
    in_block: bool,
    last_contexts: Contexts,
    failed: bool,
    remembered: Vec<(String, String)>,
}

fn sorted(map: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> =
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    entries.sort();
    entries
}

fn has_placeholder(text: &str) -> bool {
    text.contains(crate::sweetener_hygiene::PREFIX) || text.contains(crate::sweetener_scoped::OPEN)
}

impl State {
    /// The state after an item, kept for its replay; `None` when it holds a
    /// placeholder another item could print (an id of this run).
    fn take(printer: &Printer<'_>) -> Option<State> {
        let state = State {
            last_original: printer.last_original,
            hygiene: sorted(&printer.hygiene),
            imports_emitted: printer.imports_emitted,
            form_signals: printer.form_signals.clone(),
            last_form_next: printer.last_form_next,
            channel_renames: sorted(&printer.channel_renames),
            context_rejected: format!("{:?}", printer.context_rejected),
            in_block: printer.in_block,
            last_contexts: printer.last_contexts,
            failed: printer.failed,
            remembered: sorted(&crate::sweetener_hygiene::remembered()),
        };
        let placeholder = state
            .hygiene
            .iter()
            .chain(&state.form_signals)
            .chain(&state.channel_renames)
            .chain(&state.remembered)
            .any(|(a, b)| has_placeholder(a) || has_placeholder(b))
            || has_placeholder(&state.context_rejected);
        (!placeholder).then_some(state)
    }
}

/// The hash of the state the printer carries into the item at `at` (node
/// `index`), with its positions relative to the item; `None` when it holds
/// a placeholder. Hashed in place: most items are replayed, and only a
/// printed one keeps its state (`State::take`).
fn state_hash(printer: &Printer<'_>, at: usize, index: usize) -> Option<u64> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let hygiene = sorted(&printer.hygiene);
    let renames = sorted(&printer.channel_renames);
    let remembered = sorted(&crate::sweetener_hygiene::remembered());
    if hygiene
        .iter()
        .chain(&printer.form_signals)
        .chain(&renames)
        .chain(&remembered)
        .any(|(a, b)| has_placeholder(a) || has_placeholder(b))
    {
        return None;
    }
    (at as i64 - printer.last_original as i64).hash(&mut hasher);
    hygiene.hash(&mut hasher);
    printer.imports_emitted.hash(&mut hasher);
    printer.form_signals.hash(&mut hasher);
    (printer.last_form_next as i64 - index as i64).hash(&mut hasher);
    renames.hash(&mut hasher);
    if !printer.context_rejected.is_empty() {
        let rejected = format!("{:?}", printer.context_rejected);
        if has_placeholder(&rejected) {
            return None;
        }
        rejected.hash(&mut hasher);
    }
    printer.in_block.hash(&mut hasher);
    printer.context_base.hash(&mut hasher);
    printer.last_contexts.hash(&mut hasher);
    printer.failed.hash(&mut hasher);
    remembered.hash(&mut hasher);
    Some(hasher.finish())
}

/// Whether no expansion is in progress across an item boundary: nothing
/// half-printed, no stack, no trace still waiting for its span, and no
/// placeholder in the names an item could reuse. Traces and names only
/// grow, and a trace once placed stays placed, so each is checked once.
fn quiet(recorder: &mut Recorder, printer: &Printer<'_>) -> bool {
    let calm = printer.depth == 0
        && !printer.printing_core
        && printer.pending_captures.is_none()
        && printer.pending_pieces.is_none()
        && BUDGET.with(|budget| budget.borrow().is_none())
        && ACTIVE_EXPANSIONS.with(|active| active.borrow().is_empty())
        && CURRENT_INVOCATION.with(|stack| stack.borrow().is_empty())
        && LITERAL_SINK.with(|sink| sink.borrow().is_none())
        && !crate::sweetener_hygiene::scope_active()
        && crate::sweetener_scoped::lets_empty();
    if !calm {
        return false;
    }
    let placed = TRACES.with(|traces| {
        let traces = traces.borrow();
        let from = recorder.traces_placed.min(traces.len());
        let more = traces[from..]
            .iter()
            .take_while(|(_, level)| *level == Some(0))
            .count();
        recorder.traces_placed = from + more;
        recorder.traces_placed == traces.len()
    });
    let added = &printer.user_names.added;
    if !recorder.added_placeholder {
        let from = recorder.added_checked.min(added.len());
        recorder.added_placeholder = added[from..].iter().any(|name| has_placeholder(name));
        recorder.added_checked = added.len();
    }
    placed && !recorder.added_placeholder
}

/// One printed item, as the next expansion replays it. Positions are in the
/// text of the expansion that logged it; virtual positions are relative to
/// the start of the item's own output.
#[derive(Clone, Debug)]
struct Item {
    /// The item's first node and how many nodes it took.
    start: usize,
    taken: usize,
    window: (usize, usize),
    key: u64,
    names: Vec<(String, bool)>,
    charges: [u64; 5],
    current: String,
    /// `last_form_next` after the item, relative to its first node.
    form_next: i64,
    /// The state after the item.
    after: State,
    names_added: Vec<String>,
    out: String,
    segments: Vec<Segment>,
    fine: Vec<(u32, u32, u32)>,
    diagnostics: Vec<Diagnostic>,
    /// Per diagnostic: its positions are in this file, moved by an edit. A
    /// nested printer's are in its own text, which no edit moves.
    diagnostic_in_file: Vec<bool>,
    consumed: Vec<(usize, usize, Vec<String>)>,
    traces: Vec<crate::ExpansionTrace>,
    /// Per trace: its `rule_start` is a position of this file (a rule the
    /// file defines), moved by an edit. A rule a `#syntax` definition made
    /// points into the text that definition rendered, which no edit moves.
    rule_in_file: Vec<bool>,
    invoked: Vec<String>,
    unused: Vec<Unused>,
    regions: Vec<(usize, (String, String, bool))>,
    probes: Vec<Probe>,
}

/// An item in a log: its record, and how far the text it was printed in
/// has moved since (edits before it). A replayed item is logged again by
/// reference, its shift added to. One whose traces name a rule of this file
/// is logged again with its positions rewritten: that rule moves on its own.
#[derive(Clone)]
struct Logged {
    item: Arc<Item>,
    shift: i64,
}

/// The items one pass over one text printed.
struct Log {
    fingerprint: u64,
    source: Arc<str>,
    items: HashMap<usize, Logged>,
}

type Slot = (String, u64);

fn logs() -> &'static Mutex<HashMap<Slot, Arc<Log>>> {
    static LOGS: OnceLock<Mutex<HashMap<Slot, Arc<Log>>>> = OnceLock::new();
    LOGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Logs of at most this many passes are kept; an editor works on a few files.
const MAX_LOGS: usize = 256;

pub(crate) fn clear() {
    if let Ok(mut logs) = logs().lock() {
        logs.clear();
    }
}

thread_local! {
    static OFF_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with incremental expansion off on this thread when `!on` (a
/// test compares the two).
pub(crate) fn with_incremental<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let saved = OFF_HERE.with(|off| off.replace(!on));
    let out = f();
    OFF_HERE.with(|off| off.set(saved));
    out
}

/// Items replayed and items printed by every logged pass so far.
static HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PRINTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn stats() -> (usize, usize) {
    (
        HITS.load(std::sync::atomic::Ordering::Relaxed),
        PRINTED.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Whether incremental expansion is off (`SWEETENER_NO_INCREMENTAL=1`).
fn disabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    OFF_HERE.with(std::cell::Cell::get)
        || *OFF
            .get_or_init(|| std::env::var_os("SWEETENER_NO_INCREMENTAL").is_some_and(|v| v != "0"))
}

/// The one range of the old text an edit replaced: the old text and the
/// new one share `prefix` bytes at the start and `suffix` at the end.
#[derive(Clone, Copy, Debug)]
struct Edit {
    prefix: usize,
    old_end: usize,
    new_end: usize,
}

impl Edit {
    fn between(old: &str, new: &str) -> Edit {
        let (a, b) = (old.as_bytes(), new.as_bytes());
        let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let room = a.len().min(b.len()) - prefix;
        let suffix = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(room)
            .take_while(|(x, y)| x == y)
            .count();
        Edit {
            prefix,
            old_end: a.len() - suffix,
            new_end: b.len() - suffix,
        }
    }

    fn delta(self) -> i64 {
        self.new_end as i64 - self.old_end as i64
    }

    /// A position of the old text in the new one; `None` in or at the edges
    /// of the replaced range.
    fn forward(self, old: usize) -> Option<usize> {
        if old < self.prefix {
            Some(old)
        } else if old > self.old_end {
            Some((old as i64 + self.delta()) as usize)
        } else {
            None
        }
    }

    /// A position of the new text in the old one.
    fn backward(self, new: usize) -> Option<usize> {
        if new < self.prefix {
            Some(new)
        } else if new > self.new_end {
            Some((new as i64 - self.delta()) as usize)
        } else {
            None
        }
    }

    /// How a whole old range moves: unchanged before the edit, shifted
    /// after it. `None` when it touches the edit.
    fn range_shift(self, (start, end): (usize, usize)) -> Option<i64> {
        if end <= self.prefix {
            Some(0)
        } else if start >= self.old_end && (start > self.prefix || self.old_end > self.prefix) {
            Some(self.delta())
        } else {
            None
        }
    }
}

/// What one pass reads of the whole file, computed once.
pub(super) struct Recorder {
    slot: Slot,
    fingerprint: u64,
    source: Arc<str>,
    previous: Option<(Arc<Log>, Edit)>,
    /// Open JSX tags before each top-level index (`jsx_child_context`).
    jsx: Vec<i32>,
    /// Top-level `for syntax` imports before each index.
    imports_before: Vec<u32>,
    /// Local macro heads, by position.
    heads: Vec<(usize, String)>,
    regions: Vec<crate::sweetener_context::Region>,
    /// `regions` sorted by start, and the furthest end among the first `i`.
    by_start: Vec<crate::sweetener_context::Region>,
    reach: Vec<usize>,
    /// Traces known placed, names known free of placeholders.
    traces_placed: usize,
    added_checked: usize,
    added_placeholder: bool,
    /// The macros and classes of a later generation (an item defined a
    /// macro), hashed once per generation.
    generation: (u32, u64),
    /// Where the rules this file defines start.
    file_rules: std::collections::HashSet<u32>,
    items: HashMap<usize, Logged>,
    hits: usize,
    printed: usize,
    /// Why items were not replayed, when `SWEETENER_ITEMS_DEBUG` is set.
    misses: HashMap<&'static str, usize>,
}

/// `hash_debug` of each element, in an order that does not depend on the
/// list's: the imported macros come out of a map.
fn hash_debug_set<T: std::fmt::Debug>(values: &[T], hasher: &mut impl Hasher) {
    let mut each: Vec<u64> = values
        .iter()
        .map(|value| {
            let mut one = std::collections::hash_map::DefaultHasher::new();
            hash_debug(value, &mut one);
            one.finish()
        })
        .collect();
    each.sort_unstable();
    each.hash(hasher);
}

/// Hash the `Debug` print of `value` with every position left out: the
/// `start`, `end` and `at` of a local definition move with any edit above
/// it. An item's output carries such a position only as a position, which
/// the replay moves by the edit, or as an order, which the key holds.
fn hash_debug(value: &impl std::fmt::Debug, hasher: &mut impl Hasher) {
    use std::fmt::Write;
    let mut text = String::new();
    let _ = write!(text, "{value:?}");
    let mut rest = text.as_str();
    while let Some(found) = ["start: ", "end: ", "at: "]
        .iter()
        .filter_map(|label| rest.find(label).map(|at| at + label.len()))
        .min()
    {
        hasher.write(rest[..found].as_bytes());
        rest = rest[found..].trim_start_matches(|ch: char| ch.is_ascii_digit());
    }
    hasher.write(rest.as_bytes());
}

impl Recorder {
    /// The recorder for the pass `printer` is about to print, or `None`
    /// when incremental expansion is off for it.
    pub(super) fn new(
        printer: &Printer<'_>,
        filename: &str,
        phase: crate::Phase,
        pass: bool,
    ) -> Option<Box<Recorder>> {
        if disabled() || crate::sweetener_budget::counters().is_none() {
            return None;
        }
        let nodes = printer.file_nodes;
        let mut kind = std::collections::hash_map::DefaultHasher::new();
        pass.hash(&mut kind);
        let filter = OWNER_FILTER.with(|filter| {
            filter.borrow().as_ref().map(|set| {
                let mut owners: Vec<String> = set.iter().cloned().collect();
                owners.sort();
                owners
            })
        });
        filter.hash(&mut kind);
        let slot = (filename.to_owned(), kind.finish());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        filename.hash(&mut hasher);
        format!("{phase:?}").hash(&mut hasher);
        slot.1.hash(&mut hasher);
        hash_debug_set(&printer.macros, &mut hasher);
        hash_debug_set(&printer.classes, &mut hasher);
        hash_debug(&printer.runtime_imports, &mut hasher);
        crate::sweetener_reader::extra_operators().hash(&mut hasher);
        // No item adds to them; they are the modules the pass loaded.
        printer.dependencies.hash(&mut hasher);
        crate::sweetener_forms::imported_syntax_bindings(nodes).hash(&mut hasher);
        format!("{:?}", crate::sweetener_budget::configured()).hash(&mut hasher);
        let mut parameters: Vec<String> =
            SYNTAX_PARAMETERS.with(|set| set.borrow().iter().cloned().collect());
        parameters.sort();
        parameters.hash(&mut hasher);
        let mut unknown: Vec<(String, String)> = UNKNOWN_PARAMETERIZED.with(|map| {
            map.borrow()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        });
        unknown.sort();
        unknown.hash(&mut hasher);
        let mut owners: Vec<String> =
            IMPORT_OWNERS.with(|set| set.borrow().iter().cloned().collect());
        owners.sort();
        owners.hash(&mut hasher);
        let mut heads: Vec<(usize, String)> = LOCAL_HEADS.with(|heads| {
            heads
                .borrow()
                .iter()
                .map(|(name, at)| (*at as usize, name.clone()))
                .collect()
        });
        heads.sort();
        heads
            .iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .hash(&mut hasher);
        let fingerprint = hasher.finish();

        let source: Arc<str> = Arc::from(printer.source);
        let previous = logs()
            .lock()
            .ok()
            .and_then(|logs| logs.get(&slot).cloned())
            .map(|log| {
                let edit = Edit::between(&log.source, &source);
                (log, edit)
            });

        let mut jsx = vec![0i32; nodes.len() + 1];
        {
            // `jsx_child_context(nodes, i)` for every `i` in one walk: the
            // walk is the same for every `i`, it only stops sooner.
            let mut diff = vec![0i32; nodes.len() + 2];
            let mut cursor = 0usize;
            while cursor < nodes.len() {
                let open = super::operator_is(nodes.get(cursor), "<");
                let slash = super::operator_is(nodes.get(cursor + 1), "/");
                if open && !slash {
                    diff[cursor + 1] += 1;
                    cursor += 1;
                } else if open && slash {
                    diff[cursor + 1] -= 1;
                    cursor += 2;
                } else {
                    cursor += 1;
                }
            }
            let mut running = 0;
            for (index, slot) in jsx.iter_mut().enumerate() {
                running += diff[index];
                *slot = running;
            }
        }
        let mut imports_before = vec![0u32; nodes.len() + 1];
        for index in 0..nodes.len() {
            imports_before[index + 1] =
                imports_before[index] + u32::from(super::is_syntax_import_at(nodes, index));
        }
        let regions = crate::sweetener_context::regions(nodes, crate::sweetener_context::MODULE);
        let mut recorder = Box::new(Recorder {
            slot,
            fingerprint,
            source,
            previous,
            jsx,
            imports_before,
            heads,
            by_start: {
                let mut sorted = regions.clone();
                sorted.sort_by_key(|(start, end, _)| (*start, *end));
                sorted
            },
            reach: Vec::new(),
            regions,
            traces_placed: 0,
            added_checked: 0,
            added_placeholder: false,
            generation: (0, 0),
            file_rules: printer
                .macros
                .iter()
                .flat_map(|macro_| &macro_.rules)
                .filter(|rule| rule.owner.is_none())
                .filter_map(|rule| rule.origin.map(|span| span.start))
                .collect(),
            items: HashMap::new(),
            hits: 0,
            printed: 0,
            misses: HashMap::new(),
        });
        recorder.index_regions();
        Some(recorder)
    }

    fn index_regions(&mut self) {
        let mut furthest = 0;
        self.reach = std::iter::once(0)
            .chain(self.by_start.iter().map(|(_, end, _)| {
                furthest = furthest.max(*end);
                furthest
            }))
            .collect();
    }

    /// The context regions the lazy lookup would build (`contexts_at`).
    pub(super) fn regions(&self) -> Vec<crate::sweetener_context::Region> {
        self.regions.clone()
    }

    fn window(&self, nodes: &[Syntax], index: usize, next: usize, last: usize) -> (usize, usize) {
        let first = index.saturating_sub(LOOK_BEHIND);
        let start = super::node_start(&nodes[first]).min(last);
        let tail = (next + LOOK_AHEAD).min(nodes.len()) - 1;
        let mut end = super::node_span(&nodes[tail.max(index)]).end as usize;
        end = end.max(super::node_span(&nodes[next.saturating_sub(1).max(index)]).end as usize);
        end = self.source[end.min(self.source.len())..]
            .find('\n')
            .map_or(self.source.len(), |at| end + at + 1);
        (start.min(end), end)
    }

    fn key(&self, state: u64, nodes: &[Syntax], index: usize, window: (usize, usize)) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.hash(&mut hasher);
        self.fingerprint.hash(&mut hasher);
        let at = super::node_start(&nodes[index]);
        (at - window.0).hash(&mut hasher);
        // The window's text is not hashed: an item is replayed only where
        // its whole window is outside the edit, so the text is the same.
        (window.1 - window.0).hash(&mut hasher);
        index.min(LOOK_BEHIND + 1).hash(&mut hasher);
        self.jsx[index].hash(&mut hasher);
        self.imports_before[index].hash(&mut hasher);
        // A macro defined below this item is held (`held_name`).
        for (head, name) in &self.heads {
            if *head > at {
                name.hash(&mut hasher);
            }
        }
        // The regions overlapping the window: those starting before its
        // end, back to where no earlier one reaches into it.
        let upto = self
            .by_start
            .partition_point(|(start, _, _)| *start < window.1);
        let mut at = upto;
        while at > 0 && self.reach[at] > window.0 {
            at -= 1;
            let (start, end, contexts) = &self.by_start[at];
            if *end > window.0 {
                (*start as i64 - window.0 as i64).hash(&mut hasher);
                (*end as i64 - window.0 as i64).hash(&mut hasher);
                contexts.hash(&mut hasher);
            }
        }
        hasher.finish()
    }
}

/// Print (or replay) the item at `index` of the file's top level.
pub(super) fn step(printer: &mut Printer<'_>, nodes: &[Syntax], index: usize) -> usize {
    let Some(mut recorder) = printer.items.take() else {
        return printer.print_step(nodes, index);
    };
    let next = run(&mut recorder, printer, nodes, index);
    printer.items = Some(recorder);
    next
}

/// The hash of the printer's macros and classes when an item changed them.
fn generation_hash(recorder: &mut Recorder, printer: &Printer<'_>) -> u64 {
    if printer.macro_generation != recorder.generation.0 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_debug_set(&printer.macros, &mut hasher);
        hash_debug_set(&printer.classes, &mut hasher);
        recorder.generation = (printer.macro_generation, hasher.finish());
    }
    recorder.generation.1
}

fn run(
    recorder: &mut Recorder,
    printer: &mut Printer<'_>,
    nodes: &[Syntax],
    index: usize,
) -> usize {
    let at = super::node_start(&nodes[index]);
    let generation = printer.macro_generation;
    let macros = generation_hash(recorder, printer);
    let state = if quiet(recorder, printer) {
        state_hash(printer, at, index)
    } else {
        None
    };
    let counters = crate::sweetener_budget::counters();
    let (Some(state), Some(counters)) = (state, counters) else {
        *recorder.misses.entry("busy state").or_default() += 1;
        recorder.printed += 1;
        return printer.print_step(nodes, index);
    };
    let state = state ^ macros.rotate_left(17);
    let last_before = printer.last_original;
    if let Some(next) = replay(recorder, printer, nodes, index, at, state, &counters) {
        recorder.hits += 1;
        return next;
    }
    recorder.printed += 1;

    // Print it, and log what it did.
    let out_len = printer.out.len();
    let segments_len = printer.segments.len();
    let last_segment = printer.segments.last().cloned();
    let fine_len = printer.fine.len();
    let diagnostics_len = printer.diagnostics.len();
    let consumed_len = printer.consumed.len();
    let traces_len = TRACES.with(|traces| traces.borrow().len());
    // The item's own owners and observations, not what it added to the
    // file's: an owner or a probe an earlier item already had is the item's
    // too, and a replay must bring it where that earlier item changed.
    let invoked_before = INVOKED_OWNERS.with(|set| std::mem::take(&mut *set.borrow_mut()));
    let unused_len = crate::sweetener_scoped::unused_len();
    let probes_before = crate::sweetener_cache::take_probes();
    let added_len = printer.user_names.added.len();
    NAME_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
    let next = printer.print_step(nodes, index);
    let names = NAME_LOG
        .with(|log| log.borrow_mut().take())
        .unwrap_or_default();
    let invoked: Vec<String> = INVOKED_OWNERS.with(|set| {
        let mut set = set.borrow_mut();
        let mine: Vec<String> = set.iter().cloned().collect();
        set.extend(invoked_before);
        mine
    });
    let probes = crate::sweetener_cache::take_probes();
    crate::sweetener_cache::record_probes(&probes_before);
    crate::sweetener_cache::record_probes(&probes);

    // An item that defined a macro changed what every later item sees.
    if printer.macro_generation != generation || !quiet(recorder, printer) {
        return next;
    }
    let Some(after) = State::take(printer) else {
        return next;
    };
    let Some(after_counters) = crate::sweetener_budget::counters() else {
        return next;
    };
    if next <= index || next > nodes.len() || !printer.context_rejected.is_empty() {
        return next;
    }
    // A verbatim copy that continued the segment before the item is the
    // item's own first segment, merged again on replay.
    let mut segments: Vec<Segment> = printer.segments[segments_len..].to_vec();
    if let (Some(was), Some(now)) = (
        &last_segment,
        printer.segments.get(segments_len.wrapping_sub(1)),
    ) && (was.virtual_end != now.virtual_end || was.original_end != now.original_end)
    {
        segments.insert(
            0,
            Segment {
                virtual_start: was.virtual_end,
                virtual_end: now.virtual_end,
                original_start: was.original_end,
                original_end: now.original_end,
                kind: SegmentKind::Verbatim,
            },
        );
    }
    let base = out_len as u32;
    for segment in &mut segments {
        segment.virtual_start -= base;
        segment.virtual_end -= base;
    }
    let out = printer.out[out_len..].to_owned();
    let traces: Vec<crate::ExpansionTrace> = TRACES.with(|traces| {
        traces.borrow()[traces_len..]
            .iter()
            .map(|(trace, _)| trace.clone())
            .collect()
    });
    let diagnostics = printer.diagnostics[diagnostics_len..].to_vec();
    let diagnostic_in_file: Vec<bool> = (diagnostics_len..printer.diagnostics.len())
        .map(|at| {
            !printer
                .nested_diagnostics
                .iter()
                .any(|(from, to)| (*from..*to).contains(&at))
        })
        .collect();
    let rule_in_file: Vec<bool> = traces
        .iter()
        .map(|trace| {
            trace
                .rule_start
                .is_some_and(|start| recorder.file_rules.contains(&start))
        })
        .collect();
    // With rules a definition made, a start could be either kind.
    if printer.macro_generation > 0 && rule_in_file.iter().any(|in_file| *in_file) {
        return next;
    }
    let window = recorder.window(nodes, index, next, last_before);
    // Text another item could print, or a placeholder of this run outside
    // the item's text, is not logged.
    let leaks = has_placeholder(&recorder.source[window.0..window.1])
        || diagnostics.iter().any(|d| has_placeholder(&d.message))
        || traces.iter().any(|t| has_placeholder(&t.macro_name));
    if leaks {
        return next;
    }
    // Every position the item keeps must be one of this file's, in the
    // window: a position elsewhere may be in another text (a macro module,
    // a nested printer's), which an edit of this file does not move.
    let inside = |pos: usize| pos >= window.0 && pos <= window.1;
    let outside = segments
        .iter()
        .flat_map(|s| [s.original_start, s.original_end])
        .chain(printer.fine[fine_len..].iter().map(|piece| piece.2))
        .chain(traces.iter().flat_map(|t| [t.start, t.end]))
        .chain(
            diagnostics
                .iter()
                .zip(&diagnostic_in_file)
                .filter(|(_, in_file)| **in_file)
                .flat_map(|(d, _)| [d.start, d.end]),
        )
        .map(|pos| pos as usize)
        .chain(
            printer.consumed[consumed_len..]
                .iter()
                .flat_map(|c| [c.0, c.1]),
        )
        .chain([after.last_original])
        .any(|pos| !inside(pos))
        || diagnostics
            .iter()
            .zip(&diagnostic_in_file)
            .any(|(d, in_file)| *in_file && !d.related.is_empty());
    if outside {
        return next;
    }
    let mut regions = Vec::new();
    for id in placeholder_ids(&out, crate::sweetener_scoped::OPEN, b'(') {
        match crate::sweetener_scoped::region(id) {
            Some(header) => regions.push((id, header)),
            None => return next,
        }
    }
    let mut charges = [0u64; 5];
    for (slot, (now, was)) in charges
        .iter_mut()
        .zip(after_counters.used.iter().zip(counters.used.iter()))
    {
        *slot = now - was;
    }
    let item = Item {
        start: at,
        taken: next - index,
        window,
        key: recorder.key(state, nodes, index, window),
        names,
        charges,
        current: after_counters.current,
        form_next: after.last_form_next as i64 - index as i64,
        after,
        names_added: printer.user_names.added[added_len..].to_vec(),
        out,
        segments,
        fine: printer.fine[fine_len..]
            .iter()
            .map(|(start, end, at)| (start - base, end - base, *at))
            .collect(),
        diagnostics,
        diagnostic_in_file,
        consumed: printer.consumed[consumed_len..].to_vec(),
        traces,
        rule_in_file,
        invoked,
        unused: crate::sweetener_scoped::unused_since(unused_len),
        regions,
        probes,
    };
    recorder.items.insert(
        at,
        Logged {
            item: Arc::new(item),
            shift: 0,
        },
    );
    next
}

/// `<prefix><digits><end>` ids in `text`.
fn placeholder_ids(text: &str, prefix: &str, end: u8) -> Vec<usize> {
    let mut ids = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(prefix) {
        let digits_at = from + found + prefix.len();
        let digits = text.as_bytes()[digits_at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0
            && text.as_bytes().get(digits_at + digits) == Some(&end)
            && let Ok(id) = text[digits_at..digits_at + digits].parse()
        {
            ids.push(id);
        }
        from = digits_at;
    }
    ids
}

/// `text` with every placeholder id replaced by a fresh one (the same old
/// id, the same new one), and how far each old offset moved.
fn renumber(
    text: &str,
    regions: &[(usize, (String, String, bool))],
) -> (String, Vec<(usize, i64)>) {
    let hygiene = crate::sweetener_hygiene::PREFIX;
    let scoped = crate::sweetener_scoped::OPEN;
    if !text.contains(hygiene) && !text.contains(scoped) {
        return (text.to_owned(), Vec::new());
    }
    let mut fresh_h: HashMap<usize, usize> = HashMap::new();
    let mut fresh_p: HashMap<usize, usize> = HashMap::new();
    let mut out = String::with_capacity(text.len() + 16);
    let mut moves = Vec::new();
    let mut shift = 0i64;
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut copied = 0;
    while index < text.len() {
        let (prefix, end, scoped_id) = if text[index..].starts_with(hygiene) {
            (hygiene, b'_', false)
        } else if text[index..].starts_with(scoped) {
            (scoped, b'(', true)
        } else {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let digits_at = index + prefix.len();
        let digits = bytes[digits_at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let id = (digits > 0 && bytes.get(digits_at + digits) == Some(&end))
            .then(|| text[digits_at..digits_at + digits].parse::<usize>().ok())
            .flatten();
        let Some(id) = id else {
            index = digits_at;
            continue;
        };
        let new = if scoped_id {
            *fresh_p.entry(id).or_insert_with(|| {
                let header = regions
                    .iter()
                    .find(|(old, _)| *old == id)
                    .map(|(_, header)| header.clone())
                    .unwrap_or_default();
                crate::sweetener_scoped::renumber_region(header)
            })
        } else {
            *fresh_h
                .entry(id)
                .or_insert_with(crate::sweetener_hygiene::fresh_id)
        };
        out.push_str(&text[copied..digits_at]);
        let printed = new.to_string();
        out.push_str(&printed);
        copied = digits_at + digits;
        shift += printed.len() as i64 - digits as i64;
        moves.push((copied, shift));
        index = copied;
    }
    out.push_str(&text[copied..]);
    (out, moves)
}

fn moved(moves: &[(usize, i64)], at: u32) -> u32 {
    let shift = moves
        .iter()
        .take_while(|(end, _)| *end <= at as usize)
        .last()
        .map_or(0, |(_, shift)| *shift);
    (at as i64 + shift) as u32
}

#[allow(clippy::too_many_arguments)]
fn replay(
    recorder: &mut Recorder,
    printer: &mut Printer<'_>,
    nodes: &[Syntax],
    index: usize,
    at: usize,
    state: u64,
    counters: &crate::sweetener_budget::Counters,
) -> Option<usize> {
    let miss = |recorder: &mut Recorder, why: &'static str| {
        *recorder.misses.entry(why).or_default() += 1;
        None
    };
    let Some((log, edit)) = recorder.previous.clone() else {
        return miss(recorder, "no log");
    };
    if log.fingerprint != recorder.fingerprint {
        return miss(recorder, "fingerprint");
    }
    let Some(old_at) = edit.backward(at) else {
        return miss(recorder, "edited");
    };
    let Some(logged) = log.items.get(&old_at).cloned() else {
        return miss(recorder, "not logged");
    };
    let item = &*logged.item;
    let next = index + item.taken;
    if next > nodes.len() {
        return None;
    }
    let window = recorder.window(nodes, index, next, printer.last_original);
    let in_log = |pos: usize| (pos as i64 + logged.shift) as usize;
    let Some(edit_shift) = edit.range_shift((in_log(item.window.0), in_log(item.window.1))) else {
        return miss(recorder, "window edited");
    };
    // From the item's own text to this one.
    let shift = logged.shift + edit_shift;
    let moved_by = |pos: usize| (pos as i64 + shift) as usize;
    if (moved_by(item.window.0), moved_by(item.window.1)) != window {
        return miss(recorder, "window moved");
    }
    if recorder.key(state, nodes, index, window) != item.key {
        return miss(recorder, "key");
    }
    if !printer.user_names.answers(&item.names) {
        return miss(recorder, "names");
    }
    for ((used, charge), limit) in counters
        .used
        .iter()
        .zip(&item.charges)
        .zip(&counters.limits)
    {
        if used.saturating_add(*charge) > *limit {
            return miss(recorder, "budget");
        }
    }
    // Every position the item kept is in its window (see `run`), so it
    // moves with it, except the start of a rule this file defines, which
    // moves by the edit (such an item is logged with no shift), and a
    // nested printer's diagnostics, in a text no edit moves.
    let mut rule_starts = Vec::new();
    for (trace, in_file) in item.traces.iter().zip(&item.rule_in_file) {
        rule_starts.push(match trace.rule_start {
            Some(start) if *in_file => match edit.forward(start as usize) {
                Some(start) => Some(start as u32),
                None => return miss(recorder, "rule edited"),
            },
            other => other,
        });
    }
    let m32 = |pos: u32| moved_by(pos as usize) as u32;

    // Apply: the text with fresh placeholder ids, the maps, the state.
    let (text, moves) = renumber(&item.out, &item.regions);
    let base = printer.out.len() as u32;
    printer.out.push_str(&text);
    for (position, segment) in item.segments.iter().enumerate() {
        let segment = Segment {
            virtual_start: base + moved(&moves, segment.virtual_start),
            virtual_end: base + moved(&moves, segment.virtual_end),
            original_start: m32(segment.original_start),
            original_end: m32(segment.original_end),
            kind: segment.kind,
        };
        if position == 0
            && segment.kind == SegmentKind::Verbatim
            && let Some(last) = printer.segments.last_mut()
            && last.kind == SegmentKind::Verbatim
            && last.virtual_end == segment.virtual_start
            && last.original_end == segment.original_start
        {
            last.virtual_end = segment.virtual_end;
            last.original_end = segment.original_end;
            continue;
        }
        printer.segments.push(segment);
    }
    printer
        .fine
        .extend(item.fine.iter().map(|(start, end, source)| {
            (
                base + moved(&moves, *start),
                base + moved(&moves, *end),
                m32(*source),
            )
        }));
    for (diagnostic, in_file) in item.diagnostics.iter().zip(&item.diagnostic_in_file) {
        let mut diagnostic = diagnostic.clone();
        if *in_file {
            // A logged item's own diagnostics carry no related locations.
            diagnostic.start = m32(diagnostic.start);
            diagnostic.end = m32(diagnostic.end);
        }
        printer.diagnostics.push(diagnostic);
    }
    printer.consumed.extend(
        item.consumed
            .iter()
            .map(|(start, end, captures)| (moved_by(*start), moved_by(*end), captures.clone())),
    );
    let traces: Vec<crate::ExpansionTrace> = item
        .traces
        .iter()
        .zip(&rule_starts)
        .map(|(trace, rule_start)| crate::ExpansionTrace {
            start: m32(trace.start),
            end: m32(trace.end),
            rule_start: *rule_start,
            ..trace.clone()
        })
        .collect();
    TRACES.with(|all| {
        all.borrow_mut()
            .extend(traces.iter().cloned().map(|trace| (trace, Some(0))));
    });
    INVOKED_OWNERS.with(|set| set.borrow_mut().extend(item.invoked.iter().cloned()));
    if !item.unused.is_empty() {
        crate::sweetener_scoped::note_unused(item.unused.clone());
    }
    crate::sweetener_cache::record_probes(&item.probes);
    crate::sweetener_budget::replay(&item.charges, &item.current);
    for name in &item.names_added {
        printer.user_names.insert(name.clone());
    }
    let after = &item.after;
    printer.last_original = moved_by(after.last_original);
    if printer.hygiene.len() != after.hygiene.len()
        || after
            .hygiene
            .iter()
            .any(|(k, v)| printer.hygiene.get(k) != Some(v))
    {
        printer.hygiene = after.hygiene.iter().cloned().collect();
    }
    printer.imports_emitted = after.imports_emitted;
    if printer.form_signals != after.form_signals {
        printer.form_signals.clone_from(&after.form_signals);
    }
    printer.last_form_next = (index as i64 + item.form_next).max(0) as usize;
    if !(printer.channel_renames.is_empty() && after.channel_renames.is_empty()) {
        printer.channel_renames = after.channel_renames.iter().cloned().collect();
    }
    printer.in_block = after.in_block;
    printer.last_contexts = after.last_contexts;
    printer.failed = after.failed;
    if !(after.remembered.is_empty() && crate::sweetener_hygiene::remembered_is_empty()) {
        crate::sweetener_hygiene::set_remembered(after.remembered.iter().cloned().collect());
    }
    // A logged item left no rejected rule (see `run`).
    printer.context_rejected.clear();

    // Log it again for the next edit.
    let relogged = if item.rule_in_file.iter().any(|in_file| *in_file) {
        let mut moved_item = item.clone();
        moved_item.window = window;
        moved_item.start = at;
        moved_item.after.last_original = moved_by(item.after.last_original);
        for segment in &mut moved_item.segments {
            segment.original_start = m32(segment.original_start);
            segment.original_end = m32(segment.original_end);
        }
        for piece in &mut moved_item.fine {
            piece.2 = m32(piece.2);
        }
        for (diagnostic, in_file) in moved_item
            .diagnostics
            .iter_mut()
            .zip(&item.diagnostic_in_file)
        {
            if *in_file {
                diagnostic.start = m32(diagnostic.start);
                diagnostic.end = m32(diagnostic.end);
            }
        }
        for consumed in &mut moved_item.consumed {
            consumed.0 = moved_by(consumed.0);
            consumed.1 = moved_by(consumed.1);
        }
        moved_item.traces = traces;
        Logged {
            item: Arc::new(moved_item),
            shift: 0,
        }
    } else {
        Logged {
            item: logged.item.clone(),
            shift,
        }
    };
    recorder.items.insert(at, relogged);
    Some(next)
}

/// Keep the log of the pass `recorder` watched, for the next expansion.
pub(super) fn finish(recorder: Box<Recorder>) {
    let Recorder {
        slot,
        fingerprint,
        source,
        previous,
        items,
        hits,
        printed,
        misses,
        ..
    } = *recorder;
    if std::env::var_os("SWEETENER_ITEMS_DEBUG").is_some() {
        eprintln!(
            "items {}: {hits} replayed, {printed} printed, misses {misses:?}",
            slot.0
        );
    }
    HITS.fetch_add(hits, std::sync::atomic::Ordering::Relaxed);
    PRINTED.fetch_add(printed, std::sync::atomic::Ordering::Relaxed);
    let replaced = logs().lock().ok().and_then(|mut logs| {
        if logs.len() >= MAX_LOGS && !logs.contains_key(&slot) {
            logs.clear();
        }
        logs.insert(
            slot,
            Arc::new(Log {
                fingerprint,
                source,
                items,
            }),
        )
    });
    // The log this one replaces is the editor's last version: freeing a
    // large one is not the edit's work.
    let large = replaced.as_ref().is_some_and(|log| log.items.len() > 512);
    if large {
        let _ = std::thread::Builder::new()
            .name("sweetener-log-free".to_owned())
            .spawn(move || drop((replaced, previous)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_moves_what_follows_it_and_nothing_inside_it() {
        let edit = Edit::between("abc def ghi", "abc dXXef ghi");
        assert_eq!((edit.prefix, edit.old_end, edit.new_end), (5, 5, 7));
        assert_eq!(edit.forward(2), Some(2));
        assert_eq!(edit.forward(5), None);
        assert_eq!(edit.forward(8), Some(10));
        assert_eq!(edit.backward(10), Some(8));
        assert_eq!(edit.backward(6), None);
        assert_eq!(edit.range_shift((0, 4)), Some(0));
        assert_eq!(edit.range_shift((6, 11)), Some(2));
        assert_eq!(edit.range_shift((3, 6)), None);
        // A deletion moves back. The two texts share `one t` and `hree`, so
        // the deleted range is `wo t` (5..9): a range ending at it stands, one
        // starting inside it does not.
        let edit = Edit::between("one two three", "one three");
        assert_eq!((edit.prefix, edit.old_end, edit.new_end), (5, 9, 5));
        assert_eq!(edit.range_shift((0, 4)), Some(0));
        assert_eq!(edit.range_shift((9, 13)), Some(-4));
        assert_eq!(edit.range_shift((8, 13)), None);
        let same = Edit::between("same", "same");
        assert_eq!(same.range_shift((0, 4)), Some(0));
    }

    #[test]
    fn a_replayed_item_gets_fresh_placeholder_ids_with_offsets_moved() {
        let text = "const __swh7_x = __swp3(__swh7_x) + __swh12_y;";
        let regions = vec![(3, ("p".to_owned(), "v".to_owned(), false))];
        let (out, moves) = renumber(text, &regions);
        let ids = placeholder_ids(&out, crate::sweetener_hygiene::PREFIX, b'_');
        assert_eq!(ids.len(), 3);
        // The same old id is the same new one; different old ids differ.
        assert_eq!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert!(!ids.contains(&7) || !ids.contains(&12));
        let region = placeholder_ids(&out, crate::sweetener_scoped::OPEN, b'(')[0];
        assert_eq!(
            crate::sweetener_scoped::region(region),
            Some(("p".to_owned(), "v".to_owned(), false))
        );
        // The end of the text moved by the change in length.
        assert_eq!(moved(&moves, text.len() as u32) as usize, out.len());
        assert_eq!(moved(&moves, 0), 0);
        let plain = "no placeholders";
        assert_eq!(renumber(plain, &[]), (plain.to_owned(), Vec::new()));
    }

    #[test]
    fn a_logged_item_is_replayed_only_where_its_names_answer_the_same() {
        let mut names = UserNames::new(["a".to_owned()].into_iter().collect());
        NAME_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
        assert!(names.contains("a"));
        assert!(!names.contains("b"));
        let log = NAME_LOG.with(|log| log.borrow_mut().take()).unwrap();
        assert_eq!(log, vec![("a".to_owned(), true), ("b".to_owned(), false)]);
        assert!(names.answers(&log));
        names.insert("b".to_owned());
        assert!(!names.answers(&log));
        // A repeated insert is logged, for a replay where the first is gone.
        names.insert("b".to_owned());
        assert_eq!(names.added, vec!["b".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn the_pass_fingerprint_leaves_positions_out_but_not_order_or_text() {
        let hash = |text: &str| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hash_debug(&text, &mut hasher);
            hasher.finish()
        };
        // `Debug` of a `&str` quotes it, so build the prints by hand.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Span {
            start: u32,
            end: u32,
        }
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Rule {
            name: &'static str,
            head: Span,
        }
        let one = |name, start, end| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hash_debug(
                &Rule {
                    name,
                    head: Span { start, end },
                },
                &mut hasher,
            );
            hasher.finish()
        };
        assert_eq!(one("m", 10, 20), one("m", 30, 40));
        assert_ne!(one("m", 10, 20), one("n", 10, 20));
        assert_ne!(hash("a"), hash("b"));
        let set = |rules: &[(&'static str, u32)]| {
            let values: Vec<Rule> = rules
                .iter()
                .map(|(name, at)| Rule {
                    name,
                    head: Span {
                        start: *at,
                        end: *at,
                    },
                })
                .collect();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hash_debug_set(&values, &mut hasher);
            hasher.finish()
        };
        assert_eq!(set(&[("a", 1), ("b", 2)]), set(&[("b", 9), ("a", 5)]));
        assert_ne!(set(&[("a", 1)]), set(&[("a", 1), ("b", 2)]));
    }
}
