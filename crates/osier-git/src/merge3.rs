//! Three-way text merge for the merge tool: a line diff3 of two versions against their common
//! base, the chunks it finds, and the merge result as the user builds it chunk by chunk.
//!
//! The result starts as the base (as PhpStorm's merge tool does). Each chunk has a state per
//! side: applying a side copies its lines into the result, ignoring it leaves the result as it
//! is. Applying the second side of a conflict appends after the first. The result's line
//! ranges follow edits the user types into it ([`Merge::set_result`]).
//!
//! Without a base (two unrelated files) the left version stands in for it and every
//! difference is a conflict, so nothing can be taken over silently.

use std::ops::Range;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, DiffTag};

/// How a chunk differs from the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    /// Only the left side changed these lines.
    Left,
    /// Only the right side changed these lines.
    Right,
    /// Both sides made the same change.
    Same,
    /// Both sides changed these lines differently.
    Conflict,
}

/// One of the two versions being merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// What happened to one side of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideState {
    /// This side did not change the chunk: nothing to apply.
    Unchanged,
    /// Waiting for the user.
    Pending,
    /// Its lines were copied into the result.
    Applied,
    /// Dismissed: the result keeps what it has.
    Ignored,
}

impl SideState {
    pub fn is_pending(self) -> bool {
        self == SideState::Pending
    }
}

/// Whitespace handling when comparing lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Whitespace {
    /// Every character counts.
    #[default]
    Exact,
    /// Leading and trailing whitespace is ignored.
    Trim,
    /// All whitespace is ignored.
    Ignore,
}

/// A run of lines that differs between the versions. Ranges are line ranges (0-based,
/// half open) into each text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub kind: ChunkKind,
    pub base: Range<usize>,
    pub left: Range<usize>,
    pub right: Range<usize>,
    /// Where the chunk is in the result now.
    pub result: Range<usize>,
    pub left_state: SideState,
    pub right_state: SideState,
}

impl Chunk {
    pub fn state(&self, side: Side) -> SideState {
        match side {
            Side::Left => self.left_state,
            Side::Right => self.right_state,
        }
    }

    fn state_mut(&mut self, side: Side) -> &mut SideState {
        match side {
            Side::Left => &mut self.left_state,
            Side::Right => &mut self.right_state,
        }
    }

    pub fn range(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    /// Nothing left to decide.
    pub fn is_resolved(&self) -> bool {
        !self.left_state.is_pending() && !self.right_state.is_pending()
    }

    pub fn is_conflict(&self) -> bool {
        self.kind == ChunkKind::Conflict
    }
}

/// One of the three texts on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Left,
    Result,
    Right,
}

/// A change to the result: replace `lines` (in the result as it was just before) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEdit {
    pub lines: Range<usize>,
    pub text: String,
}

/// The merge of two versions and the result being built.
#[derive(Debug, Clone)]
pub struct Merge {
    base: Vec<String>,
    left: Vec<String>,
    right: Vec<String>,
    result: Vec<String>,
    chunks: Vec<Chunk>,
    initial: Vec<Chunk>,
    has_base: bool,
    whitespace: Whitespace,
}

/// Lines of `text`, each with its line terminator (the last may have none). Joining them
/// gives `text` back.
pub fn split_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

fn normalize(line: &str, whitespace: Whitespace) -> String {
    match whitespace {
        Whitespace::Exact => line.to_string(),
        Whitespace::Trim => line.trim().to_string(),
        Whitespace::Ignore => line.chars().filter(|c| !c.is_whitespace()).collect(),
    }
}

/// Changed regions between two line lists: (old range, new range) pairs in order.
fn hunks(
    old: &[String],
    new: &[String],
    whitespace: Whitespace,
) -> Vec<(Range<usize>, Range<usize>)> {
    let old_keys: Vec<String> = old.iter().map(|l| normalize(l, whitespace)).collect();
    let new_keys: Vec<String> = new.iter().map(|l| normalize(l, whitespace)).collect();
    let deadline = Instant::now() + Duration::from_secs(2);
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        &old_keys,
        &new_keys,
        Some(deadline),
    );
    group_ops(&ops)
}

fn group_ops(ops: &[DiffOp]) -> Vec<(Range<usize>, Range<usize>)> {
    let mut out: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let mut open: Option<(Range<usize>, Range<usize>)> = None;
    for op in ops {
        if op.tag() == DiffTag::Equal {
            if let Some(h) = open.take() {
                out.push(h);
            }
            continue;
        }
        let (o, n) = (op.old_range(), op.new_range());
        open = Some(match open.take() {
            Some((ho, hn)) => (ho.start..o.end.max(ho.end), hn.start..n.end.max(hn.end)),
            None => (o, n),
        });
    }
    if let Some(h) = open {
        out.push(h);
    }
    out
}

fn same_lines(a: &[String], b: &[String], whitespace: Whitespace) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| normalize(x, whitespace) == normalize(y, whitespace))
}

/// Diff3: chunks where the left or right version differs from the base. Hunks of the two
/// sides that overlap or touch form one chunk.
fn diff3(base: &[String], left: &[String], right: &[String], whitespace: Whitespace) -> Vec<Chunk> {
    let lh = hunks(base, left, whitespace);
    let rh = hunks(base, right, whitespace);
    // (base range, side range, side) sorted by base start.
    let mut all: Vec<(Range<usize>, Range<usize>, Side)> = lh
        .iter()
        .map(|(b, s)| (b.clone(), s.clone(), Side::Left))
        .chain(rh.iter().map(|(b, s)| (b.clone(), s.clone(), Side::Right)))
        .collect();
    all.sort_by_key(|(b, _, side)| (b.start, b.end, *side == Side::Right));

    // Line count change of a side's hunks wholly before a base position.
    let mut chunks = Vec::new();
    let (mut left_delta, mut right_delta): (isize, isize) = (0, 0);
    let mut i = 0;
    while i < all.len() {
        let mut region = all[i].0.clone();
        let mut members = vec![i];
        let mut j = i + 1;
        while j < all.len() {
            let b = &all[j].0;
            // Overlapping or touching (an insertion at the region's end counts).
            if b.start <= region.end {
                region.end = region.end.max(b.end);
                members.push(j);
                j += 1;
            } else {
                break;
            }
        }
        let (mut has_left, mut has_right) = (false, false);
        let (mut left_change, mut right_change): (isize, isize) = (0, 0);
        for &m in &members {
            let (b, s, side) = &all[m];
            let change = s.len() as isize - b.len() as isize;
            match side {
                Side::Left => {
                    has_left = true;
                    left_change += change;
                }
                Side::Right => {
                    has_right = true;
                    right_change += change;
                }
            }
        }
        let at = |pos: usize, delta: isize| (pos as isize + delta) as usize;
        let left_range = at(region.start, left_delta)..at(region.end, left_delta + left_change);
        let right_range = at(region.start, right_delta)..at(region.end, right_delta + right_change);
        let kind = match (has_left, has_right) {
            (true, false) => ChunkKind::Left,
            (false, true) => ChunkKind::Right,
            _ => {
                if same_lines(
                    &left[left_range.clone()],
                    &right[right_range.clone()],
                    whitespace,
                ) {
                    ChunkKind::Same
                } else {
                    ChunkKind::Conflict
                }
            }
        };
        let (left_state, right_state) = match kind {
            ChunkKind::Left => (SideState::Pending, SideState::Unchanged),
            ChunkKind::Right => (SideState::Unchanged, SideState::Pending),
            ChunkKind::Same | ChunkKind::Conflict => (SideState::Pending, SideState::Pending),
        };
        chunks.push(Chunk {
            kind,
            base: region.clone(),
            left: left_range,
            right: right_range,
            result: region,
            left_state,
            right_state,
        });
        left_delta += left_change;
        right_delta += right_change;
        i = j;
    }
    chunks
}

impl Merge {
    /// Merge `left` and `right` against `base`. With no base every difference between
    /// `left` and `right` is a conflict and the result starts as `left`.
    pub fn new(base: Option<&str>, left: &str, right: &str, whitespace: Whitespace) -> Self {
        let left_lines = split_lines(left);
        let right_lines = split_lines(right);
        let (base_lines, chunks) = match base {
            Some(base) => {
                let base_lines = split_lines(base);
                let chunks = diff3(&base_lines, &left_lines, &right_lines, whitespace);
                (base_lines, chunks)
            }
            None => {
                let chunks = hunks(&left_lines, &right_lines, whitespace)
                    .into_iter()
                    .map(|(l, r)| Chunk {
                        kind: ChunkKind::Conflict,
                        base: l.clone(),
                        left: l.clone(),
                        right: r,
                        result: l,
                        left_state: SideState::Pending,
                        right_state: SideState::Pending,
                    })
                    .collect();
                (left_lines.clone(), chunks)
            }
        };
        Merge {
            result: base_lines.clone(),
            base: base_lines,
            left: left_lines,
            right: right_lines,
            initial: chunks.clone(),
            chunks,
            has_base: base.is_some(),
            whitespace,
        }
    }

    /// Recompute the chunks with another whitespace rule, starting over.
    pub fn with_whitespace(&self, whitespace: Whitespace) -> Self {
        let base = self.has_base.then(|| self.base.concat());
        Merge::new(
            base.as_deref(),
            &self.left.concat(),
            &self.right.concat(),
            whitespace,
        )
    }

    pub fn whitespace(&self) -> Whitespace {
        self.whitespace
    }

    pub fn has_base(&self) -> bool {
        self.has_base
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn lines(&self, side: Side) -> &[String] {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub fn base_lines(&self) -> &[String] {
        &self.base
    }

    pub fn result_lines(&self) -> &[String] {
        &self.result
    }

    pub fn text(&self, side: Side) -> String {
        self.lines(side).concat()
    }

    pub fn base_text(&self) -> String {
        self.base.concat()
    }

    pub fn result_text(&self) -> String {
        self.result.concat()
    }

    /// Changes (non-conflicts) and conflicts still waiting.
    pub fn pending_counts(&self) -> (usize, usize) {
        let pending = self.chunks.iter().filter(|c| !c.is_resolved());
        let (mut changes, mut conflicts) = (0, 0);
        for c in pending {
            if c.is_conflict() {
                conflicts += 1;
            } else {
                changes += 1;
            }
        }
        (changes, conflicts)
    }

    /// Every chunk decided.
    pub fn is_resolved(&self) -> bool {
        self.chunks.iter().all(Chunk::is_resolved)
    }

    /// Something was applied, ignored or typed since the start.
    pub fn is_modified(&self) -> bool {
        self.result != self.base || self.chunks != self.initial
    }

    /// Copy `side`'s lines of chunk `ix` into the result. Replaces what the chunk holds, or
    /// appends when the other side of a conflict was applied already. `None` when there is
    /// nothing to apply.
    pub fn apply(&mut self, ix: usize, side: Side) -> Option<ResultEdit> {
        let chunk = self.chunks.get(ix)?;
        if !chunk.state(side).is_pending() {
            return None;
        }
        let other = match side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        };
        let append = chunk.is_conflict() && chunk.state(other) == SideState::Applied;
        let source = chunk.range(side).clone();
        let text: Vec<String> = self.lines(side)[source].to_vec();
        let old = chunk.result.clone();
        let replace = if append {
            old.end..old.end
        } else {
            old.clone()
        };
        let mut edit = ResultEdit {
            lines: replace.clone(),
            text: text.concat(),
        };
        // Inserting after a last line without a newline: the edit gives that line one.
        if replace.start == self.result.len()
            && replace.start > 0
            && !text.is_empty()
            && !self.result[replace.start - 1].ends_with('\n')
        {
            edit.lines = replace.start - 1..replace.end;
            edit.text = format!("{}\n{}", self.result[replace.start - 1], edit.text);
        }
        let new_len = text.len();
        self.splice_result(replace.clone(), text);
        let delta = new_len as isize - replace.len() as isize;
        for (i, c) in self.chunks.iter_mut().enumerate() {
            if i > ix && c.result.start >= old.end {
                c.result = shift(&c.result, delta);
            }
        }
        let chunk = &mut self.chunks[ix];
        chunk.result = old.start..(old.end as isize + delta) as usize;
        *chunk.state_mut(side) = SideState::Applied;
        if chunk.kind == ChunkKind::Same {
            *chunk.state_mut(other) = SideState::Applied;
        }
        Some(edit)
    }

    /// Dismiss `side` of chunk `ix`. The result does not change.
    pub fn ignore(&mut self, ix: usize, side: Side) -> bool {
        let Some(chunk) = self.chunks.get_mut(ix) else {
            return false;
        };
        if !chunk.state(side).is_pending() {
            return false;
        }
        *chunk.state_mut(side) = SideState::Ignored;
        if chunk.kind == ChunkKind::Same {
            let other = match side {
                Side::Left => Side::Right,
                Side::Right => Side::Left,
            };
            *chunk.state_mut(other) = SideState::Ignored;
        }
        true
    }

    /// Mark a chunk as decided as it is (the user resolved it by hand).
    pub fn resolve(&mut self, ix: usize) -> bool {
        let Some(chunk) = self.chunks.get_mut(ix) else {
            return false;
        };
        let mut changed = false;
        for side in [Side::Left, Side::Right] {
            if chunk.state(side).is_pending() {
                *chunk.state_mut(side) = SideState::Ignored;
                changed = true;
            }
        }
        changed
    }

    /// Apply every pending non-conflicting change from `side` (`None`: both sides). The
    /// edits are in order, each against the result left by the one before.
    pub fn apply_non_conflicting(&mut self, side: Option<Side>) -> Vec<ResultEdit> {
        let mut edits = Vec::new();
        for ix in 0..self.chunks.len() {
            let c = &self.chunks[ix];
            if c.is_conflict() {
                continue;
            }
            for s in [Side::Left, Side::Right] {
                if side.is_some_and(|want| want != s) {
                    continue;
                }
                if self.chunks[ix].state(s).is_pending()
                    && let Some(edit) = self.apply(ix, s)
                {
                    edits.push(edit);
                }
            }
        }
        edits
    }

    /// Non-conflicting changes still waiting on `side` (`None`: either side).
    pub fn has_non_conflicting(&self, side: Option<Side>) -> bool {
        self.chunks.iter().any(|c| {
            !c.is_conflict()
                && match side {
                    Some(s) => c.state(s).is_pending(),
                    None => !c.is_resolved(),
                }
        })
    }

    /// Back to the start: the result is the base again and every chunk is pending.
    pub fn revert(&mut self) {
        self.result = self.base.clone();
        self.chunks = self.initial.clone();
    }

    /// The whole result is one side's version: every chunk is decided for that side.
    pub fn take_all(&mut self, side: Side) -> String {
        self.result = self.lines(side).to_vec();
        for c in &mut self.chunks {
            c.result = c.range(side).clone();
            for s in [Side::Left, Side::Right] {
                if c.state(s).is_pending() {
                    *c.state_mut(s) = if s == side {
                        SideState::Applied
                    } else {
                        SideState::Ignored
                    };
                }
            }
        }
        self.result_text()
    }

    /// The user changed the result text. Chunk ranges move with the lines around them; an
    /// edit inside or touching a chunk grows or shrinks it.
    pub fn set_result(&mut self, text: &str) {
        let new = split_lines(text);
        if new == self.result {
            return;
        }
        let regions = hunks(&self.result, &new, Whitespace::Exact);
        for c in &mut self.chunks {
            c.result = remap(&c.result, &regions);
        }
        self.result = new;
    }

    /// Index of the first undecided chunk after (`forward`) or before result line `line`,
    /// wrapping around.
    pub fn next_pending(&self, line: usize, forward: bool) -> Option<usize> {
        let pending: Vec<usize> = (0..self.chunks.len())
            .filter(|&i| !self.chunks[i].is_resolved())
            .collect();
        if pending.is_empty() {
            return None;
        }
        if forward {
            pending
                .iter()
                .copied()
                .find(|&i| self.chunks[i].result.start > line)
                .or(pending.first().copied())
        } else {
            pending
                .iter()
                .rev()
                .copied()
                .find(|&i| self.chunks[i].result.start < line)
                .or(pending.last().copied())
        }
    }

    /// A chunk's lines in one of the panes.
    pub fn chunk_range(&self, ix: usize, pane: Pane) -> Option<Range<usize>> {
        let c = self.chunks.get(ix)?;
        Some(match pane {
            Pane::Left => c.left.clone(),
            Pane::Result => c.result.clone(),
            Pane::Right => c.right.clone(),
        })
    }

    /// The line in `to` that matches (fractional) line `line` of `from`: one to one between
    /// chunks, stretched across each chunk. Used to keep the panes scrolled together.
    pub fn map_line(&self, from: Pane, to: Pane, line: f64) -> f64 {
        if from == to {
            return line;
        }
        let (mut prev_from, mut prev_to) = (0.0, 0.0);
        let mut pairs: Vec<(Range<usize>, Range<usize>)> = (0..self.chunks.len())
            .filter_map(|i| Some((self.chunk_range(i, from)?, self.chunk_range(i, to)?)))
            .collect();
        pairs.sort_by_key(|(f, _)| (f.start, f.end));
        for (f, t) in pairs {
            let (fs, fe) = (f.start as f64, f.end as f64);
            if line < fs {
                break;
            }
            if line < fe {
                let at = (line - fs) / (fe - fs);
                return t.start as f64 + at * t.len() as f64;
            }
            prev_from = fe;
            prev_to = t.end as f64;
        }
        (prev_to + (line - prev_from)).max(0.0)
    }

    fn splice_result(&mut self, range: Range<usize>, lines: Vec<String>) {
        // The line before an insertion at the very end may lack its newline; give it one so
        // the new lines start on their own line.
        if range.start == self.result.len()
            && range.start > 0
            && !lines.is_empty()
            && let Some(last) = self.result.last_mut()
            && !last.ends_with('\n')
        {
            last.push('\n');
        }
        self.result.splice(range, lines);
    }

    /// Word-level differences inside chunk `ix`: byte ranges (relative to the start of the
    /// chunk's text) of what `side` changed compared with the base.
    pub fn word_changes(&self, ix: usize, side: Side) -> Vec<Range<usize>> {
        let Some(c) = self.chunks.get(ix) else {
            return Vec::new();
        };
        let base = self.base[c.base.clone()].concat();
        let theirs = self.lines(side)[c.range(side).clone()].concat();
        word_ranges(&base, &theirs).1
    }

    /// Word-level differences in the result's copy of chunk `ix` against the pending sides:
    /// byte ranges relative to the chunk's start in the result.
    pub fn result_word_changes(&self, ix: usize) -> Vec<Range<usize>> {
        let Some(c) = self.chunks.get(ix) else {
            return Vec::new();
        };
        let Some(lines) = self.result.get(c.result.clone()) else {
            return Vec::new();
        };
        let result = lines.concat();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            if c.state(side).is_pending() {
                let other = self.lines(side)[c.range(side).clone()].concat();
                out.extend(word_ranges(&result, &other).0);
            }
        }
        out.sort_by_key(|r| (r.start, r.end));
        out.dedup();
        out
    }
}

fn shift(r: &Range<usize>, delta: isize) -> Range<usize> {
    let s = (r.start as isize + delta).max(0) as usize;
    let e = (r.end as isize + delta).max(0) as usize;
    s..e
}

/// Map a line range through changed regions (old range, new range).
fn remap(r: &Range<usize>, regions: &[(Range<usize>, Range<usize>)]) -> Range<usize> {
    let (s, e) = (r.start, r.end);
    let mut before: isize = 0;
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    let mut inside: isize = 0;
    for (old, new) in regions {
        let change = new.len() as isize - old.len() as isize;
        let insertion = old.is_empty();
        let is_before = if s == e {
            old.end <= s && !(insertion && old.start == s)
        } else {
            old.end <= s
        };
        let is_after = if s == e {
            old.start >= s && !(insertion && old.start == s)
        } else {
            old.start >= e
        };
        if is_before {
            before += change;
        } else if !is_after {
            // Overlaps the chunk.
            start = Some(start.map_or(new.start, |x: usize| x.min(new.start)));
            end = Some(end.map_or(new.end, |x: usize| x.max(new.end)));
            inside += change;
        }
    }
    let ns = (s as isize + before) as usize;
    let ne = (e as isize + before + inside).max(ns as isize) as usize;
    let ns = start.map_or(ns, |x| x.min(ns));
    let ne = end.map_or(ne, |x| x.max(ne));
    ns..ne
}

/// Words for word-level highlights: runs of letters, digits and `_`, runs of whitespace,
/// and single other characters.
fn tokens(text: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<Class> = None;
    for (i, c) in text.char_indices() {
        let k = class(c);
        if let Some(p) = prev
            && (p != k || k == Class::Other)
        {
            out.push(&text[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Changed byte ranges in `a` and in `b`, by words.
fn word_ranges(a: &str, b: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let ta = tokens(a);
    let tb = tokens(b);
    let deadline = Instant::now() + Duration::from_millis(200);
    let ops = similar::capture_diff_slices_deadline(Algorithm::Myers, &ta, &tb, Some(deadline));
    let offsets = |t: &[&str]| {
        let mut v = Vec::with_capacity(t.len() + 1);
        let mut at = 0;
        v.push(0);
        for s in t {
            at += s.len();
            v.push(at);
        }
        v
    };
    let (oa, ob) = (offsets(&ta), offsets(&tb));
    let (mut old, mut new) = (Vec::new(), Vec::new());
    for op in ops {
        if op.tag() == DiffTag::Equal {
            continue;
        }
        for i in op.old_range() {
            if !ta[i].trim().is_empty() {
                push_range(&mut old, oa[i]..oa[i + 1]);
            }
        }
        for i in op.new_range() {
            if !tb[i].trim().is_empty() {
                push_range(&mut new, ob[i]..ob[i + 1]);
            }
        }
    }
    (old, new)
}

fn push_range(v: &mut Vec<Range<usize>>, r: Range<usize>) {
    if let Some(last) = v.last_mut()
        && last.end == r.start
    {
        last.end = r.end;
        return;
    }
    v.push(r);
}

/// A unified diff of two texts, as git prints one (without the `diff --git` line), for the
/// diff view of `osier diff`.
pub fn unified_patch(
    old_name: &str,
    new_name: &str,
    old: &str,
    new: &str,
    context: usize,
) -> String {
    let diff = similar::TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .deadline(Instant::now() + Duration::from_secs(5))
        .diff_lines(old, new);
    diff.unified_diff()
        .context_radius(context)
        .missing_newline_hint(true)
        .header(&format!("a/{old_name}"), &format!("b/{new_name}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "{\n  \"name\": \"Testing App\",\n  \"version\": \"1.0.0\",\n  \"debug\": false,\n  \"timezone\": \"UTC\",\n  \"ttl\": 3600,\n  \"host\": \"localhost\",\n  \"port\": 1025\n}\n";
    const LEFT: &str = "{\n  \"name\": \"Testing App\",\n  \"version\": \"2.0.0\",\n  \"debug\": true,\n  \"timezone\": \"UTC\",\n  \"ttl\": 600,\n  \"host\": \"localhost\",\n  \"port\": 1025\n}\n";
    const RIGHT: &str = "{\n  \"name\": \"Testing App\",\n  \"version\": \"1.1.0\",\n  \"debug\": false,\n  \"timezone\": \"UTC\",\n  \"ttl\": 7200,\n  \"host\": \"localhost\",\n  \"port\": 2525\n}\n";

    fn merge() -> Merge {
        Merge::new(Some(BASE), LEFT, RIGHT, Whitespace::Exact)
    }

    #[test]
    fn finds_conflicts_and_one_sided_changes() {
        let m = merge();
        let kinds: Vec<ChunkKind> = m.chunks().iter().map(|c| c.kind).collect();
        // version+debug (adjacent, both changed version), ttl, port (right only).
        assert_eq!(
            kinds,
            vec![ChunkKind::Conflict, ChunkKind::Conflict, ChunkKind::Right]
        );
        assert_eq!(m.chunks()[0].base, 2..4);
        assert_eq!(m.chunks()[0].left, 2..4);
        // The right side changed only the version line; its range still spans the chunk.
        assert_eq!(m.chunks()[0].right, 2..4);
        assert_eq!(m.chunks()[2].left_state, SideState::Unchanged);
        assert_eq!(m.pending_counts(), (1, 2));
        assert_eq!(m.result_text(), BASE);
        assert!(!m.is_modified());
    }

    #[test]
    fn same_change_on_both_sides_is_not_a_conflict() {
        let m = Merge::new(
            Some("a\nb\nc\n"),
            "a\nB\nc\n",
            "a\nB\nc\n",
            Whitespace::Exact,
        );
        assert_eq!(m.chunks().len(), 1);
        assert_eq!(m.chunks()[0].kind, ChunkKind::Same);
        let mut m = m;
        m.apply(0, Side::Right);
        assert!(m.is_resolved());
        assert_eq!(m.result_text(), "a\nB\nc\n");
    }

    #[test]
    fn apply_replaces_then_appends_the_other_side() {
        let mut m = merge();
        let e = m.apply(1, Side::Left).unwrap();
        assert_eq!(e.lines, 5..6);
        assert_eq!(e.text, "  \"ttl\": 600,\n");
        assert!(!m.chunks()[1].is_resolved());
        let e = m.apply(1, Side::Right).unwrap();
        assert_eq!(e.lines, 6..6, "second side appends");
        assert_eq!(m.chunks()[1].result, 5..7);
        assert!(m.chunks()[1].is_resolved());
        // The chunk after moved down a line.
        assert_eq!(m.chunks()[2].result, 8..9);
        assert!(
            m.result_text()
                .contains("\"ttl\": 600,\n  \"ttl\": 7200,\n")
        );
        assert!(m.is_modified());
    }

    #[test]
    fn ignore_keeps_the_result() {
        let mut m = merge();
        assert!(m.ignore(1, Side::Left));
        assert!(m.ignore(1, Side::Right));
        assert!(m.chunks()[1].is_resolved());
        assert_eq!(m.result_text(), BASE);
        assert!(!m.ignore(1, Side::Right), "already decided");
    }

    #[test]
    fn apply_all_non_conflicting_then_counts() {
        let mut m = merge();
        assert!(m.has_non_conflicting(Some(Side::Right)));
        assert!(!m.has_non_conflicting(Some(Side::Left)));
        let edits = m.apply_non_conflicting(None);
        assert_eq!(edits.len(), 1);
        assert!(m.result_text().contains("\"port\": 2525"));
        assert_eq!(m.pending_counts(), (0, 2));
        m.revert();
        assert_eq!(m.result_text(), BASE);
        assert_eq!(m.pending_counts(), (1, 2));
    }

    #[test]
    fn edits_replay_on_the_text() {
        // The edits a view applies one by one rebuild the model's result exactly.
        let mut m = merge();
        let mut text: Vec<String> = split_lines(BASE);
        let mut all = vec![m.apply(0, Side::Right).unwrap()];
        all.extend(m.apply_non_conflicting(None));
        all.push(m.apply(0, Side::Left).unwrap());
        for e in all {
            text.splice(e.lines, split_lines(&e.text));
        }
        assert_eq!(text.concat(), m.result_text());
    }

    #[test]
    fn typed_lines_move_the_chunks() {
        let mut m = merge();
        // A new line typed above the first chunk.
        let edited = BASE.replacen("{\n", "{\n  \"new\": 1,\n", 1);
        m.set_result(&edited);
        assert_eq!(m.chunks()[0].result, 3..5);
        assert_eq!(m.chunks()[2].result, 8..9);
        // Typing inside the ttl chunk keeps it in place and the same size.
        let edited2 = edited.replace("3600", "3601");
        m.set_result(&edited2);
        assert_eq!(m.chunks()[1].result, 6..7);
        // Deleting the chunk's line empties it.
        let edited3 = edited2.replace("  \"ttl\": 3601,\n", "");
        m.set_result(&edited3);
        assert_eq!(m.chunks()[1].result, 6..6);
        assert_eq!(m.chunks()[2].result, 7..8);
        // Applying into the emptied chunk inserts at that spot.
        m.apply(1, Side::Right);
        assert!(
            m.result_text()
                .contains("\"timezone\": \"UTC\",\n  \"ttl\": 7200,\n  \"host\"")
        );
    }

    #[test]
    fn typing_into_an_empty_chunk_grows_it() {
        // Left deleted line b: the chunk is empty in the result after applying left.
        let mut m = Merge::new(Some("a\nb\nc\n"), "a\nc\n", "a\nb\nc\n", Whitespace::Exact);
        assert_eq!(m.chunks()[0].kind, ChunkKind::Left);
        m.apply(0, Side::Left);
        assert_eq!(m.result_text(), "a\nc\n");
        assert_eq!(m.chunks()[0].result, 1..1);
        m.set_result("a\nx\nc\n");
        assert_eq!(m.chunks()[0].result, 1..2);
    }

    #[test]
    fn insertions_and_end_of_file() {
        // Right adds lines at the end of a file without a final newline.
        let mut m = Merge::new(Some("a\nb"), "a\nb", "a\nb\nc\n", Whitespace::Exact);
        assert_eq!(m.chunks().len(), 1);
        m.apply(0, Side::Right);
        assert_eq!(m.result_text(), "a\nb\nc\n");
        // Both sides insert at the same spot: a conflict.
        let m = Merge::new(Some("a\nb\n"), "a\nx\nb\n", "a\ny\nb\n", Whitespace::Exact);
        assert_eq!(m.chunks().len(), 1);
        assert_eq!(m.chunks()[0].kind, ChunkKind::Conflict);
        assert_eq!(m.chunks()[0].result, 1..1);
    }

    #[test]
    fn two_way_merge_makes_every_difference_a_conflict() {
        let m = Merge::new(None, "a\nb\nc\n", "a\nB\nc\nd\n", Whitespace::Exact);
        assert!(!m.has_base());
        assert_eq!(m.result_text(), "a\nb\nc\n");
        assert!(m.chunks().iter().all(Chunk::is_conflict));
        assert_eq!(m.chunks().len(), 2);
        assert!(!m.has_non_conflicting(None));
        let mut m = m;
        m.apply(1, Side::Right);
        m.apply(0, Side::Right);
        assert_eq!(m.result_text(), "a\nB\nc\nd\n");
    }

    #[test]
    fn whitespace_rules() {
        let m = Merge::new(Some("a\nb\n"), "a\n  b\n", "a\nb\n", Whitespace::Exact);
        assert_eq!(m.chunks().len(), 1);
        let m = m.with_whitespace(Whitespace::Trim);
        assert!(m.chunks().is_empty());
        let m = Merge::new(
            Some("f(a, b)\n"),
            "f(a,b)\n",
            "f(a, b)\n",
            Whitespace::Ignore,
        );
        assert!(m.chunks().is_empty());
    }

    #[test]
    fn take_all_and_next_pending() {
        let mut m = merge();
        assert_eq!(m.next_pending(0, true), Some(0));
        assert_eq!(m.next_pending(2, true), Some(1));
        assert_eq!(m.next_pending(100, true), Some(0), "wraps");
        assert_eq!(m.next_pending(0, false), Some(2), "wraps back");
        assert_eq!(m.take_all(Side::Left), LEFT);
        assert!(m.is_resolved());
        assert_eq!(m.next_pending(0, true), None);
    }

    #[test]
    fn word_changes_point_at_the_changed_words() {
        let m = merge();
        let chunk = &m.chunks()[1];
        let left = m.lines(Side::Left)[chunk.left.clone()].concat();
        let words: Vec<&str> = m
            .word_changes(1, Side::Left)
            .into_iter()
            .map(|r| &left[r])
            .collect();
        assert_eq!(words, vec!["600"]);
        let result = m.result_lines()[chunk.result.clone()].concat();
        let words: Vec<&str> = m
            .result_word_changes(1)
            .into_iter()
            .map(|r| &result[r])
            .collect();
        assert_eq!(words, vec!["3600"]);
    }

    #[test]
    fn lines_map_between_panes() {
        let mut m = Merge::new(
            Some("a\nb\nc\nd\n"),
            "a\nX\nY\nZ\nc\nd\n",
            "a\nb\nc\nd\n",
            Whitespace::Exact,
        );
        // Left replaced b (line 1) with three lines.
        assert_eq!(m.map_line(Pane::Left, Pane::Result, 0.0), 0.0);
        assert_eq!(m.map_line(Pane::Left, Pane::Result, 2.5), 1.5);
        assert_eq!(m.map_line(Pane::Left, Pane::Right, 5.0), 3.0);
        assert_eq!(m.map_line(Pane::Right, Pane::Left, 3.0), 5.0);
        m.apply(0, Side::Left);
        assert_eq!(m.map_line(Pane::Left, Pane::Result, 5.0), 5.0);
    }

    #[test]
    fn unified_patch_reads_like_git() {
        let p = unified_patch("x.txt", "x.txt", "a\nb\n", "a\nc\n", 3);
        assert_eq!(p, "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n");
        assert_eq!(unified_patch("x", "x", "a\n", "a\n", 3), "");
    }

    #[test]
    fn identical_and_empty_inputs() {
        let m = Merge::new(Some(""), "", "", Whitespace::Exact);
        assert!(m.chunks().is_empty());
        assert!(m.is_resolved());
        let m = Merge::new(Some("x\n"), "x\n", "x\n", Whitespace::Exact);
        assert!(m.chunks().is_empty());
        // Both sides added a file (empty base).
        let m = Merge::new(Some(""), "a\n", "b\n", Whitespace::Exact);
        assert_eq!(m.chunks().len(), 1);
        assert!(m.chunks()[0].is_conflict());
    }
}
