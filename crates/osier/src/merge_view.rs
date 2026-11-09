//! The three-way merge tool: your version, the result and their version side by side, like
//! PhpStorm's merge dialog.
//!
//! Three code editors; the outer two are read only, the result in the middle is editable.
//! Lines are not padded to line up: each pane shows its own text, and bands drawn in the two
//! gutters connect a chunk's lines across panes. Scrolling one pane scrolls the others to the
//! matching line ([`Merge::map_line`]). Fills, bands and the per-chunk buttons are positioned
//! from each editor's last layout (`text_bounds`, `line_height`), so they are painted after the
//! editors in the same frame.
//!
//! The view knows nothing about repositories: it takes texts and labels and reports what the
//! user chose through [`MergeEvent`]. The app writes the result into the working tree and
//! marks the file resolved; `osier merge` writes it to the output file and exits.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::merge3::{ChunkKind, Merge, Pane, Side, SideState, Whitespace};

use crate::test_ids::TestId as _;

/// Width of each gutter between two panes, where the bands and chunk buttons go.
const GUTTER: f32 = 52.;

/// What to merge.
#[derive(Debug, Clone)]
pub struct MergeInput {
    /// Shown in the result's header and used to pick the syntax highlighting.
    pub path: String,
    /// The common ancestor; `None` merges two unrelated files (every difference is a conflict).
    pub base: Option<String>,
    pub left: String,
    pub right: String,
    pub left_label: String,
    pub right_label: String,
}

/// Where the view is shown; decides the footer and the expand button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeHost {
    /// The app's diff pane: Save keeps the file conflicted, Apply resolves it.
    App,
    /// `osier merge`: a window of its own; Cancel exits without writing.
    Window,
}

/// What the user asked for; the host does the writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeEvent {
    /// Write this text and mark the file resolved.
    Apply(String),
    /// Write this text and keep the file conflicted (App only).
    Save(String),
    /// Close without writing (Window only).
    Cancel,
    /// Expand to the whole window, or back (App only).
    ToggleExpand,
    /// The previous or next conflicted file (App, while expanded).
    PreviousFile,
    NextFile,
}

const PANES: [Pane; 3] = [Pane::Left, Pane::Result, Pane::Right];

/// Where an editor's lines were drawn in the last frame, in window coordinates.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct PaneGeometry {
    /// The pane's box.
    bounds: Bounds<Pixels>,
    /// Top of line 0 (already moved by the scroll offset).
    text_top: Pixels,
    line_height: Pixels,
}

impl PaneGeometry {
    fn y(&self, line: usize) -> Pixels {
        self.text_top + self.line_height * line as f32
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Geometry {
    body: Bounds<Pixels>,
    panes: [Option<PaneGeometry>; 3],
}

/// One chunk as the painting code needs it, captured at render time.
#[derive(Debug, Clone)]
struct ChunkPaint {
    ranges: [Range<usize>; 3],
    /// Which panes show the chunk filled (unresolved and changed there).
    filled: [bool; 3],
    /// Band in the left gutter, the right gutter.
    bands: [bool; 2],
    color: Hsla,
    resolved: bool,
}

pub struct MergeView {
    input: MergeInput,
    host: MergeHost,
    merge: Merge,
    /// Left, result, right.
    editors: [Entity<EditorState>; 3],
    words: [Option<TextDecorationCollection>; 3],
    geometry: Rc<RefCell<Geometry>>,
    /// Geometry the buttons were placed with, to tell when they need placing again.
    placed: Geometry,
    /// The pane the user scrolls or types in; the others follow it.
    driver: Rc<Cell<usize>>,
    last_scroll: [Pixels; 3],
    sync_scroll: bool,
    highlight_words: bool,
    /// Shown by the App host: the view fills the window.
    pub expanded: bool,
    /// Which conflicted file this is (1-based) and how many there are, for the file
    /// navigation shown while expanded.
    pub file_position: Option<(usize, usize)>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for MergeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeView")
            .field("path", &self.input.path)
            .field("host", &self.host)
            .finish()
    }
}

impl EventEmitter<MergeEvent> for MergeView {}

impl Focusable for MergeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Byte offset of the start of `line` in `text` (the end of the text past the last line).
fn line_byte(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    text.len()
}

/// "1 change, 2 conflicts" for the toolbar.
fn counter(changes: usize, conflicts: usize) -> String {
    if changes == 0 && conflicts == 0 {
        return "All changes processed".into();
    }
    let part = |n: usize, one: &str, many: &str| match n {
        0 => format!("No {many}"),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    let first = part(changes, "change", "changes");
    let second = part(conflicts, "conflict", "conflicts").to_lowercase();
    format!("{first}, {second}")
}

impl MergeView {
    pub fn new(
        input: MergeInput,
        host: MergeHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let merge = Merge::new(
            input.base.as_deref(),
            &input.left,
            &input.right,
            Whitespace::Exact,
        );
        let language = crate::edit_pane::language_for(&input.path);
        let texts = [input.left.clone(), merge.result_text(), input.right.clone()];
        let editors = texts.map(|text| {
            cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .line_number(true)
                    .soft_wrap(false)
                    .folding(false)
                    .default_value(text)
            })
        });
        let words = [0, 1, 2].map(|i| {
            Some(editors[i].update(cx, |s, cx| s.create_decorations_collection(Vec::new(), cx)))
        });
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &editors[1],
            window,
            |this, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.result_changed(cx);
                }
            },
        ));
        for (i, editor) in editors.iter().enumerate() {
            subscriptions.push(cx.observe_in(editor, window, move |this, _, window, cx| {
                this.pane_changed(i, window, cx)
            }));
        }
        let mut view = MergeView {
            input,
            host,
            merge,
            editors,
            words,
            geometry: Rc::new(RefCell::new(Geometry::default())),
            placed: Geometry::default(),
            driver: Rc::new(Cell::new(1)),
            last_scroll: [px(0.); 3],
            sync_scroll: true,
            highlight_words: true,
            expanded: false,
            file_position: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        view.refresh_words(cx);
        view
    }

    #[cfg(test)]
    pub fn merge(&self) -> &Merge {
        &self.merge
    }

    /// The result as it is in the editor (the user may have typed in it).
    pub fn result_text(&self, cx: &App) -> String {
        self.editors[1].read(cx).value().to_string()
    }

    pub fn result_editor(&self) -> &Entity<EditorState> {
        &self.editors[1]
    }

    /// Something was applied, ignored or typed.
    #[cfg(test)]
    pub fn is_modified(&self) -> bool {
        self.merge.is_modified()
    }

    // ----- user actions -----

    pub fn apply(&mut self, ix: usize, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.merge.apply(ix, side) {
            self.write_edits(vec![edit], window, cx);
        }
        self.changed(cx);
    }

    pub fn ignore(&mut self, ix: usize, side: Side, cx: &mut Context<Self>) {
        if self.merge.ignore(ix, side) {
            self.changed(cx);
        }
    }

    /// Apply the pending non-conflicting changes of one side, or both (`None`).
    pub fn apply_non_conflicting(
        &mut self,
        side: Option<Side>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edits = self.merge.apply_non_conflicting(side);
        if !edits.is_empty() {
            self.write_edits(edits, window, cx);
        }
        self.changed(cx);
    }

    /// Start over: the result is the base again.
    pub fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.merge.revert();
        let text = self.merge.result_text();
        self.editors[1].update(cx, |s, cx| {
            let scroll = s.scroll_offset();
            s.replace_all(text, window, cx);
            s.set_scroll_offset(scroll, cx);
        });
        self.changed(cx);
    }

    pub fn confirm_revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            "Revert all changes?",
            "Everything applied, ignored or typed is undone. The result goes back to how the merge started.",
            "Revert",
            ButtonVariant::Danger,
            |this, window, cx| this.revert(window, cx),
            window,
            cx,
        );
    }

    /// Accept Left / Accept Right: the whole file from one side, then apply.
    pub fn accept(&mut self, side: Side, cx: &mut Context<Self>) {
        let text = self.merge.take_all(side);
        cx.emit(MergeEvent::Apply(text));
    }

    /// Apply: hand the result over. Asks first when something is still undecided.
    pub fn request_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (changes, conflicts) = self.merge.pending_counts();
        if changes + conflicts == 0 {
            let text = self.result_text(cx);
            cx.emit(MergeEvent::Apply(text));
            return;
        }
        let what = match (changes, conflicts) {
            (0, c) => crate::format::count(c, "conflict"),
            (c, 0) => crate::format::count(c, "change"),
            (a, b) => format!(
                "{} and {}",
                crate::format::count(a, "change"),
                crate::format::count(b, "conflict")
            ),
        };
        self.confirm(
            "Apply with undecided changes?",
            format!(
                "{what} still waiting. The result is saved exactly as shown in the middle and the file counts as resolved."
            ),
            "Apply anyway",
            ButtonVariant::Primary,
            |this, _, cx| {
                let text = this.result_text(cx);
                cx.emit(MergeEvent::Apply(text));
            },
            window,
            cx,
        );
    }

    /// Select the next (or previous) undecided chunk and scroll to it.
    pub fn go_to_change(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.editors[1].read(cx).cursor_position().line as usize;
        let Some(ix) = self.merge.next_pending(line, forward) else {
            return;
        };
        self.scroll_to_chunk(ix, window, cx);
    }

    pub fn set_whitespace(
        &mut self,
        whitespace: Whitespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.merge.whitespace() == whitespace || self.merge.is_modified() {
            return;
        }
        self.merge = self.merge.with_whitespace(whitespace);
        let text = self.merge.result_text();
        self.editors[1].update(cx, |s, cx| {
            let scroll = s.scroll_offset();
            s.replace_all(text, window, cx);
            s.set_scroll_offset(scroll, cx);
        });
        self.changed(cx);
    }

    pub fn toggle_sync_scroll(&mut self, cx: &mut Context<Self>) {
        self.sync_scroll = !self.sync_scroll;
        cx.notify();
    }

    pub fn toggle_highlight_words(&mut self, cx: &mut Context<Self>) {
        self.highlight_words = !self.highlight_words;
        self.refresh_words(cx);
        cx.notify();
    }

    // ----- internals -----

    #[allow(clippy::too_many_arguments)]
    fn confirm(
        &mut self,
        title: &'static str,
        body: impl Into<SharedString>,
        ok: &'static str,
        variant: ButtonVariant,
        on_ok: impl Fn(&mut MergeView, &mut Window, &mut Context<MergeView>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let body: SharedString = body.into();
        let on_ok = Rc::new(on_ok);
        window.open_dialog(cx, move |dialog, _, _| {
            let (weak, on_ok) = (weak.clone(), on_ok.clone());
            dialog
                .title(title)
                .child(v_flex().pb_1().w_full().child(body.clone()))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok)
                        .ok_variant(variant)
                        .show_cancel(true),
                )
                .footer(crate::dialog_buttons::DialogButtons::new(ok).variant(variant))
                .on_ok(move |_, window, cx| {
                    let (weak, on_ok) = (weak.clone(), on_ok.clone());
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| on_ok(this, window, cx));
                    });
                    true
                })
        });
    }

    /// Put the model's edits into the result editor, one after the other, keeping the scroll
    /// position. Each edit goes through the editor's undo history.
    fn write_edits(
        &mut self,
        edits: Vec<osier_git::merge3::ResultEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editors[1].update(cx, |s, cx| {
            let scroll = s.scroll_offset();
            for edit in edits {
                let text = s.value();
                let start = line_byte(&text, edit.lines.start);
                let end = line_byte(&text, edit.lines.end);
                s.set_selected_range(start..end, cx);
                s.replace(edit.text, window, cx);
            }
            s.set_scroll_offset(scroll, cx);
        });
    }

    fn result_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.result_text(cx);
        if text != self.merge.result_text() {
            self.merge.set_result(&text);
            self.changed(cx);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_words(cx);
        cx.notify();
    }

    /// Word-level highlights inside the undecided chunks.
    fn refresh_words(&mut self, cx: &mut Context<Self>) {
        let mut per_pane: [Vec<TextDecoration>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        if self.highlight_words {
            let theme = cx.theme();
            let texts = [
                self.merge.text(Side::Left),
                self.merge.result_text(),
                self.merge.text(Side::Right),
            ];
            for (ix, chunk) in self.merge.chunks().iter().enumerate() {
                if chunk.is_resolved() {
                    continue;
                }
                let color = kind_color(chunk.kind, theme).opacity(0.35);
                let style = HighlightStyle {
                    background_color: Some(color),
                    ..Default::default()
                };
                let mut add = |pane: usize, line: usize, ranges: Vec<Range<usize>>| {
                    let base = line_byte(&texts[pane], line);
                    for r in ranges {
                        per_pane[pane]
                            .push(TextDecoration::new(base + r.start..base + r.end, style));
                    }
                };
                if chunk.left_state.is_pending() {
                    add(0, chunk.left.start, self.merge.word_changes(ix, Side::Left));
                }
                if chunk.right_state.is_pending() {
                    add(
                        2,
                        chunk.right.start,
                        self.merge.word_changes(ix, Side::Right),
                    );
                }
                add(1, chunk.result.start, self.merge.result_word_changes(ix));
            }
        }
        for (i, decorations) in per_pane.into_iter().enumerate() {
            if let Some(collection) = &self.words[i] {
                collection.set(decorations, cx);
            }
        }
    }

    /// An editor notified: it scrolled, or the user typed or moved the caret in it.
    fn pane_changed(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.editors[i].read(cx).focus_handle(cx).is_focused(window) {
            self.driver.set(i);
        }
        let offset = self.editors[i].read(cx).scroll_offset().y;
        if offset == self.last_scroll[i] {
            return;
        }
        self.last_scroll[i] = offset;
        cx.notify();
        if self.sync_scroll && self.driver.get() == i {
            self.follow(i, offset, cx);
        }
    }

    /// Scroll the other panes to match pane `i` scrolled to `offset`: the line in the middle
    /// of `i` goes to the middle of each other pane.
    fn follow(&mut self, i: usize, offset: Pixels, cx: &mut Context<Self>) {
        let geometry = *self.geometry.borrow();
        let Some(line_height) = self.editors[i].read(cx).line_height() else {
            return;
        };
        let height = |p: usize| {
            geometry.panes[p]
                .map(|g| g.bounds.size.height)
                .unwrap_or(px(600.))
        };
        let middle = (f32::from(-offset) + f32::from(height(i)) / 2.) / f32::from(line_height);
        for (j, pane) in PANES.iter().enumerate() {
            if j == i {
                continue;
            }
            let line = self.merge.map_line(PANES[i], *pane, f64::from(middle)) as f32;
            let target = -(line * f32::from(line_height) - f32::from(height(j)) / 2.).max(0.);
            self.editors[j].update(cx, |s, cx| {
                let x = s.scroll_offset().x;
                s.set_scroll_offset(point(x, px(target)), cx);
            });
        }
    }

    fn scroll_to_chunk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.merge.chunk_range(ix, Pane::Result) else {
            return;
        };
        let geometry = *self.geometry.borrow();
        let line_height = self.editors[1].read(cx).line_height().unwrap_or(px(20.));
        let height = geometry.panes[1]
            .map(|g| g.bounds.size.height)
            .unwrap_or(px(600.));
        let target =
            -(f32::from(line_height) * range.start as f32 - f32::from(height) / 4.).max(0.);
        self.editors[1].update(cx, |s, cx| {
            s.set_cursor_position(
                gpui_kit::component::input::Position::new(range.start as u32, 0),
                window,
                cx,
            );
            let x = s.scroll_offset().x;
            s.set_scroll_offset(point(x, px(target)), cx);
        });
        self.driver.set(1);
        self.follow(1, px(target), cx);
        cx.notify();
    }

    fn chunk_paints(&self, cx: &App) -> Vec<ChunkPaint> {
        let theme = cx.theme();
        self.merge
            .chunks()
            .iter()
            .map(|c| {
                let resolved = c.is_resolved();
                // A side shows its change until it is applied or ignored.
                let left = c.left_state.is_pending();
                let right = c.right_state.is_pending();
                ChunkPaint {
                    ranges: [c.left.clone(), c.result.clone(), c.right.clone()],
                    filled: [left, !resolved, right],
                    bands: [left, right],
                    color: kind_color(c.kind, theme),
                    resolved,
                }
            })
            .collect()
    }
}

/// `color`, unless its hue is too close to `against` to tell them apart (warm palettes such as
/// a Dux Shell wallpaper scheme can make blue a pink); then `fallback`.
fn distinct(color: Hsla, against: Hsla, fallback: Hsla) -> Hsla {
    let d = (color.h - against.h).abs();
    if d.min(1. - d) < 0.12 || color.s < 0.15 {
        fallback
    } else {
        color
    }
}

fn kind_color(kind: ChunkKind, theme: &gpui_kit::component::Theme) -> Hsla {
    match kind {
        ChunkKind::Conflict => theme.red,
        // Catppuccin's green and blue as fallbacks.
        ChunkKind::Same => distinct(theme.green, theme.red, rgb(0xa6e3a1).into()),
        ChunkKind::Left | ChunkKind::Right => distinct(theme.blue, theme.red, rgb(0x89b4fa).into()),
    }
}

/// Dashed horizontal line, for resolved chunk edges.
fn dashed(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: Hsla) {
    let mut b = PathBuilder::stroke(px(1.)).dash_array(&[px(3.), px(3.)]);
    b.move_to(from);
    b.line_to(to);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// Fills (and edges) of every chunk in one pane, full width.
fn paint_pane(window: &mut Window, g: &PaneGeometry, pane: usize, chunks: &[ChunkPaint]) {
    let (left, right) = (g.bounds.left(), g.bounds.right());
    for c in chunks {
        let r = &c.ranges[pane];
        let (top, bottom) = (g.y(r.start), g.y(r.end));
        if bottom < g.bounds.top() - px(2.) || top > g.bounds.bottom() + px(2.) {
            continue;
        }
        if c.resolved {
            dashed(
                window,
                point(left, top),
                point(right, top),
                c.color.opacity(0.6),
            );
            if r.end > r.start {
                dashed(
                    window,
                    point(left, bottom),
                    point(right, bottom),
                    c.color.opacity(0.6),
                );
            }
        } else if c.filled[pane] {
            if r.is_empty() {
                window.paint_quad(fill(
                    Bounds::new(point(left, top - px(1.)), size(right - left, px(2.))),
                    c.color.opacity(0.8),
                ));
            } else {
                window.paint_quad(fill(
                    Bounds::new(point(left, top), size(right - left, bottom - top)),
                    c.color.opacity(0.16),
                ));
            }
        }
    }
}

/// A band joining lines `a` (on the gutter's left edge `x0`) to `b` (on its right edge `x1`).
fn paint_band(
    window: &mut Window,
    x0: Pixels,
    x1: Pixels,
    a: (Pixels, Pixels),
    b: (Pixels, Pixels),
    color: Hsla,
) {
    let mid = x0 + (x1 - x0) / 2.;
    if a.0 == a.1 && b.0 == b.1 {
        let mut p = PathBuilder::stroke(px(2.));
        p.move_to(point(x0, a.0));
        p.cubic_bezier_to(point(x1, b.0), point(mid, a.0), point(mid, b.0));
        if let Ok(path) = p.build() {
            window.paint_path(path, color.opacity(0.8));
        }
        return;
    }
    let mut p = PathBuilder::fill();
    p.move_to(point(x0, a.0));
    p.cubic_bezier_to(point(x1, b.0), point(mid, a.0), point(mid, b.0));
    p.line_to(point(x1, b.1.max(b.0 + px(1.))));
    p.cubic_bezier_to(
        point(x0, a.1.max(a.0 + px(1.))),
        point(mid, b.1),
        point(mid, a.1),
    );
    p.close();
    if let Ok(path) = p.build() {
        window.paint_path(path, color.opacity(0.16));
    }
}

impl Render for MergeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let chunks = Rc::new(self.chunk_paints(cx));
        let geometry = self.geometry.clone();
        self.placed = *geometry.borrow();
        let placed = self.placed;
        let (changes, conflicts) = self.merge.pending_counts();
        let has_left = self.merge.has_non_conflicting(Some(Side::Left));
        let has_right = self.merge.has_non_conflicting(Some(Side::Right));
        let has_any = self.merge.has_non_conflicting(None);
        let pending = changes + conflicts > 0;
        let whitespace = self.merge.whitespace();
        let modified = self.merge.is_modified();
        let weak = cx.entity().downgrade();

        let toolbar = h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .items_center()
            .flex_wrap()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("merge-prev")
                    .test_id("merge-prev")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ArrowUp)
                    .tooltip("Previous change (shift-F7)")
                    .disabled(!pending)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.go_to_change(false, window, cx)),
                    ),
            )
            .child(
                Button::new("merge-next")
                    .test_id("merge-next")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ArrowDown)
                    .tooltip("Next change (F7)")
                    .disabled(!pending)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.go_to_change(true, window, cx)),
                    ),
            )
            .child(Separator::vertical().h_4())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Apply non-conflicting changes:"),
            )
            .child(
                Button::new("merge-apply-left")
                    .test_id("merge-apply-left")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ChevronsRight)
                    .label("Left")
                    .disabled(!has_left)
                    .tooltip("Apply the changes only your side made")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(Some(Side::Left), window, cx)
                    })),
            )
            .child(
                Button::new("merge-apply-all")
                    .test_id("merge-apply-all")
                    .ghost()
                    .xsmall()
                    .label("All")
                    .disabled(!has_any)
                    .tooltip("Apply every change that does not conflict, from both sides")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(None, window, cx)
                    })),
            )
            .child(
                Button::new("merge-apply-right")
                    .test_id("merge-apply-right")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ChevronsLeft)
                    .label("Right")
                    .disabled(!has_right)
                    .tooltip("Apply the changes only their side made")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(Some(Side::Right), window, cx)
                    })),
            )
            .child(Separator::vertical().h_4())
            .child(
                Button::new("merge-revert")
                    .test_id("merge-revert")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Undo2)
                    .disabled(!modified)
                    .tooltip("Revert all changes")
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_revert(window, cx))),
            )
            .when(!self.merge.has_base(), |this| {
                this.child(
                    Tag::warning()
                        .xsmall()
                        .child("No common base: every difference is a conflict"),
                )
            })
            .child(div().flex_1())
            .when(self.expanded, |this| {
                this.when_some(self.file_position, |this, (n, total)| {
                    this.child(
                        Button::new("merge-prev-file")
                            .test_id("merge-prev-file")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::ChevronLeft)
                            .disabled(total < 2)
                            .tooltip("Previous conflicted file")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::PreviousFile))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("File {n} of {total}")),
                    )
                    .child(
                        Button::new("merge-next-file")
                            .test_id("merge-next-file")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::ChevronRight)
                            .disabled(total < 2)
                            .tooltip("Next conflicted file")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::NextFile))),
                    )
                    .child(Separator::vertical().h_4())
                })
            })
            .child(
                div()
                    .test_id("merge-counter")
                    .text_xs()
                    .text_color(if pending {
                        theme.foreground
                    } else {
                        theme.success
                    })
                    .child(counter(changes, conflicts)),
            )
            .child({
                let weak = weak.clone();
                let sync = self.sync_scroll;
                let words = self.highlight_words;
                Button::new("merge-settings")
                    .test_id("merge-settings")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Settings)
                    .dropdown_menu(move |mut menu, _, _| {
                        let w = weak.clone();
                        menu = menu.item(
                            PopupMenuItem::new("Synchronize scrolling")
                                .checked(sync)
                                .on_click(move |_, _, cx| {
                                    let _ = w.update(cx, |this, cx| this.toggle_sync_scroll(cx));
                                }),
                        );
                        menu = menu.separator().label("Ignore differences");
                        for (label, value) in [
                            ("None", Whitespace::Exact),
                            ("Trim whitespace", Whitespace::Trim),
                            ("Ignore whitespace", Whitespace::Ignore),
                        ] {
                            let w = weak.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(whitespace == value)
                                    .disabled(modified && whitespace != value)
                                    .on_click(move |_, window, cx| {
                                        let _ = w.update(cx, |this, cx| {
                                            this.set_whitespace(value, window, cx)
                                        });
                                    }),
                            );
                        }
                        let w = weak.clone();
                        menu.separator().item(
                            PopupMenuItem::new("Highlight changed words")
                                .checked(words)
                                .on_click(move |_, _, cx| {
                                    let _ =
                                        w.update(cx, |this, cx| this.toggle_highlight_words(cx));
                                }),
                        )
                    })
            })
            .when(self.host == MergeHost::App, |this| {
                this.child(
                    Button::new("merge-expand")
                        .test_id("merge-expand")
                        .ghost()
                        .xsmall()
                        .icon(if self.expanded {
                            Lucide::Minimize2
                        } else {
                            Lucide::Maximize2
                        })
                        .tooltip(if self.expanded {
                            "Back to the normal layout (Escape)"
                        } else {
                            "Expand: hide the sidebar and file list"
                        })
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::ToggleExpand))),
                )
            });

        let label = |icon: bool, text: SharedString, extra: Option<SharedString>| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .items_center()
                .when(icon, |this| {
                    this.child(Icon::new(Lucide::Lock).xsmall().text_color(muted))
                })
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .when(extra.is_some(), |d| d.flex_none())
                        .when(extra.is_none(), |d| d.truncate())
                        .child(text),
                )
                .when_some(extra, |this, e| {
                    this.child(div().text_xs().text_color(muted).truncate().child(e))
                })
        };
        let headers = h_flex()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .child(label(true, self.input.left_label.clone().into(), None))
            .child(div().w(px(GUTTER)))
            .child(label(
                false,
                "Result".into(),
                Some(self.input.path.clone().into()),
            ))
            .child(div().w(px(GUTTER)))
            .child(label(true, self.input.right_label.clone().into(), None));

        // A pane: the editor with an overlay that paints the chunk fills from the editor's
        // layout of this same frame, and records it for the gutters and buttons.
        let pane = |i: usize, readonly: bool, chunks: Rc<Vec<ChunkPaint>>| {
            let state = self.editors[i].clone();
            let geometry = geometry.clone();
            div()
                .test_id(["merge-left", "merge-result", "merge-right"][i])
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .child(
                    Editor::new(&self.editors[i])
                        .readonly(readonly)
                        .bordered(false)
                        .size_full(),
                )
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            let s = state.read(cx);
                            let (Some(text), Some(line_height)) =
                                (s.text_bounds(), s.line_height())
                            else {
                                return;
                            };
                            let g = PaneGeometry {
                                bounds,
                                text_top: text.origin.y,
                                line_height,
                            };
                            geometry.borrow_mut().panes[i] = Some(g);
                            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                paint_pane(window, &g, i, &chunks)
                            });
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
        };

        // Gutter overlay: drawn after all three panes, so their geometry is from this frame.
        let overlay = {
            let geometry = geometry.clone();
            let chunks = chunks.clone();
            let driver = self.driver.clone();
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut g = geometry.borrow_mut();
                    g.body = bounds;
                    let snapshot = *g;
                    drop(g);
                    // Which pane the user scrolls or clicks in leads the scrolling.
                    let panes = snapshot.panes;
                    let pick = move |p: Point<Pixels>, driver: &Cell<usize>| {
                        for (i, pg) in panes.iter().enumerate() {
                            if pg.is_some_and(|g| g.bounds.contains(&p)) {
                                driver.set(i);
                            }
                        }
                    };
                    let d = driver.clone();
                    window.on_mouse_event(move |e: &ScrollEvent, phase, _, _| {
                        if phase == DispatchPhase::Capture {
                            pick(e.position, &d);
                        }
                    });
                    let d = driver.clone();
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, _, _| {
                        if phase == DispatchPhase::Capture {
                            pick(e.position, &d);
                        }
                    });
                    let [Some(l), Some(c), Some(r)] = snapshot.panes else {
                        return;
                    };
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for chunk in chunks.iter() {
                            if chunk.bands[0] {
                                let a = &chunk.ranges[0];
                                let b = &chunk.ranges[1];
                                paint_band(
                                    window,
                                    l.bounds.right(),
                                    c.bounds.left(),
                                    (l.y(a.start), l.y(a.end)),
                                    (c.y(b.start), c.y(b.end)),
                                    chunk.color,
                                );
                            }
                            if chunk.bands[1] {
                                let a = &chunk.ranges[1];
                                let b = &chunk.ranges[2];
                                paint_band(
                                    window,
                                    c.bounds.right(),
                                    r.bounds.left(),
                                    (c.y(a.start), c.y(a.end)),
                                    (r.y(b.start), r.y(b.end)),
                                    chunk.color,
                                );
                            }
                        }
                    });
                    // The buttons were placed from an older layout (the panes scrolled):
                    // draw again so they catch up.
                    if snapshot != placed {
                        window.request_animation_frame();
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };

        // Chunk buttons in the gutters, from the last frame's layout.
        let mut buttons: Vec<AnyElement> = Vec::new();
        if let [Some(l), Some(_), Some(r)] = placed.panes {
            let body = placed.body;
            for (ix, chunk) in self.merge.chunks().iter().enumerate() {
                for (side, g, range) in [
                    (Side::Left, l, chunk.left.clone()),
                    (Side::Right, r, chunk.right.clone()),
                ] {
                    if !chunk.state(side).is_pending() {
                        continue;
                    }
                    let y = g.y(range.start) - body.top();
                    if y < px(-4.) || y > body.size.height - px(4.) {
                        continue;
                    }
                    let x = match side {
                        Side::Left => l.bounds.right() - body.left(),
                        Side::Right => r.bounds.left() - body.left() - px(GUTTER),
                    };
                    let key = match side {
                        Side::Left => "left",
                        Side::Right => "right",
                    };
                    let apply =
                        Button::new(SharedString::from(format!("merge-chunk-apply-{key}-{ix}")))
                            .test_id(format!("merge-chunk-apply:{key}:{ix}"))
                            .ghost()
                            .xsmall()
                            .icon(match side {
                                Side::Left => Lucide::ChevronsRight,
                                Side::Right => Lucide::ChevronsLeft,
                            })
                            .tooltip(
                                if chunk.is_conflict()
                                    && chunk.state(match side {
                                        Side::Left => Side::Right,
                                        Side::Right => Side::Left,
                                    }) == SideState::Applied
                                {
                                    "Append below the other side"
                                } else {
                                    "Apply"
                                },
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.apply(ix, side, window, cx)
                            }));
                    let ignore =
                        Button::new(SharedString::from(format!("merge-chunk-ignore-{key}-{ix}")))
                            .test_id(format!("merge-chunk-ignore:{key}:{ix}"))
                            .ghost()
                            .xsmall()
                            .icon(Lucide::X)
                            .tooltip("Ignore")
                            .on_click(cx.listener(move |this, _, _, cx| this.ignore(ix, side, cx)));
                    let pair = match side {
                        Side::Left => h_flex().child(ignore).child(apply),
                        Side::Right => h_flex().child(apply).child(ignore),
                    };
                    buttons.push(
                        div()
                            .absolute()
                            .left(x)
                            .top(y)
                            .w(px(GUTTER))
                            .justify_center()
                            .flex()
                            .child(pair)
                            .into_any_element(),
                    );
                }
            }
        }

        let body = h_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(pane(0, true, chunks.clone()))
            .child(div().w(px(GUTTER)).h_full().bg(theme.background))
            .child(pane(1, false, chunks.clone()))
            .child(div().w(px(GUTTER)).h_full().bg(theme.background))
            .child(pane(2, true, chunks.clone()))
            .child(overlay)
            .children(buttons);

        let footer = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("merge-accept-left")
                    .test_id("merge-accept-left")
                    .outline()
                    .small()
                    .label("Accept Left")
                    .tooltip("Use the left version for the whole file")
                    .on_click(cx.listener(|this, _, _, cx| this.accept(Side::Left, cx))),
            )
            .child(
                Button::new("merge-accept-right")
                    .test_id("merge-accept-right")
                    .outline()
                    .small()
                    .label("Accept Right")
                    .tooltip("Use the right version for the whole file")
                    .on_click(cx.listener(|this, _, _, cx| this.accept(Side::Right, cx))),
            )
            .child(div().flex_1())
            .map(|this| match self.host {
                MergeHost::App => this.child(
                    Button::new("merge-save")
                        .test_id("merge-save")
                        .outline()
                        .small()
                        .label("Save")
                        .tooltip("Write the result to the file; it stays conflicted")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let text = this.result_text(cx);
                            cx.emit(MergeEvent::Save(text))
                        })),
                ),
                MergeHost::Window => this.child(
                    Button::new("merge-cancel")
                        .test_id("merge-cancel")
                        .outline()
                        .small()
                        .label("Cancel")
                        .tooltip("Close without writing the result")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::Cancel))),
                ),
            })
            .child(
                Button::new("merge-apply")
                    .test_id("merge-apply")
                    .primary()
                    .small()
                    .label("Apply")
                    .tooltip("Write the result and mark the file resolved (ctrl-enter)")
                    .on_click(cx.listener(|this, _, window, cx| this.request_apply(window, cx))),
            );

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("MergeView")
            .size_full()
            .min_w_0()
            .bg(theme.background)
            // Before the editors see them: F7 / shift-F7 move between changes.
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                let m = &k.modifiers;
                if k.key == "f7" && !m.control && !m.alt && !m.platform {
                    this.go_to_change(!m.shift, window, cx);
                    cx.stop_propagation();
                }
            }))
            // ctrl-enter applies. The code editor binds it to Enter { secondary } (a newline),
            // so take that before the editor does; in the app it may arrive as Commit instead.
            .capture_action(cx.listener(
                |this, e: &gpui_kit::component::input::Enter, window, cx| {
                    if e.secondary {
                        cx.stop_propagation();
                        this.request_apply(window, cx);
                    }
                },
            ))
            .on_action(
                cx.listener(|this, _: &crate::app::CommitStaged, window, cx| {
                    this.request_apply(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| {
                    if this.expanded {
                        cx.emit(MergeEvent::ToggleExpand);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .child(toolbar)
            .child(headers)
            .child(body)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::{counter, distinct, line_byte};
    use gpui_kit::{Hsla, rgb};

    #[test]
    fn colours_that_look_alike_fall_back() {
        let red: Hsla = rgb(0xf38ba8).into();
        let blue: Hsla = rgb(0x89b4fa).into();
        let pink: Hsla = rgb(0xdf8997).into();
        let fallback: Hsla = rgb(0x0000ff).into();
        assert_eq!(distinct(blue, red, fallback), blue);
        assert_eq!(distinct(pink, red, fallback), fallback);
    }

    #[test]
    fn counter_text() {
        assert_eq!(counter(1, 2), "1 change, 2 conflicts");
        assert_eq!(counter(0, 1), "No changes, 1 conflict");
        assert_eq!(counter(3, 0), "3 changes, no conflicts");
        assert_eq!(counter(0, 0), "All changes processed");
    }

    #[test]
    fn line_offsets() {
        assert_eq!(line_byte("a\nbb\nc", 0), 0);
        assert_eq!(line_byte("a\nbb\nc", 1), 2);
        assert_eq!(line_byte("a\nbb\nc", 2), 5);
        assert_eq!(line_byte("a\nbb\nc", 3), 6);
        assert_eq!(line_byte("a\nbb\n", 2), 5);
    }
}
