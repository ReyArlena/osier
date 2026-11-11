//! Paints one row of the commit graph.

use gpui_kit::*;
use osier_git::graph::{Edge, GraphRow};

pub const LANE_WIDTH: f32 = 14.0;
pub const NODE_RADIUS: f32 = 5.0;
/// Row height of the log list; the canvas needs a definite height to lay out lines.
pub const ROW_HEIGHT: f32 = 30.0;

/// Colours for lanes, cycling.
pub fn lane_colors(cx: &App) -> [Hsla; 5] {
    use gpui_kit::component::ActiveTheme as _;
    let t = cx.theme();
    [t.chart_1, t.chart_2, t.chart_3, t.chart_4, t.chart_5]
}

/// How commits that are not on any remote are drawn (Settings > Git > Log).
#[derive(Debug, Clone, Copy)]
pub enum UnpushedPaint {
    /// Faded towards this background colour. Mixed, not transparent: a see-through node
    /// would show the lane line through it.
    Dim(Hsla),
    Colour(Hsla),
    /// Not marked: drawn like any other commit.
    Off,
}

impl UnpushedPaint {
    fn apply(self, color: Hsla) -> Hsla {
        match self {
            UnpushedPaint::Dim(background) => background.blend(color.opacity(0.4)),
            UnpushedPaint::Colour(c) => c,
            UnpushedPaint::Off => color,
        }
    }
}

/// How visible the text of an unpushed commit's row stays with `UnpushedPaint::Dim`: a little
/// faded next to its faded graph node, still easy to read.
pub const UNPUSHED_TEXT_OPACITY: f32 = 0.7;

/// Which parts of a row belong to unpushed commits: this row's node (`this`), and per edge
/// whether it is a link from an unpushed commit to its parent, for the edges leaving this row
/// (`outgoing`, one per `row.edges`) and those arriving from the previous row (`incoming`).
/// A link keeps its mark while it passes other rows; other branches' lines are left alone
/// (see `osier_git::graph::marked_edges`).
#[derive(Debug, Clone)]
pub struct UnpushedMarks {
    pub paint: UnpushedPaint,
    pub this: bool,
    pub outgoing: Vec<bool>,
    pub incoming: Vec<bool>,
}

/// Canvas element for a row: incoming edges from the previous row, outgoing edges, node.
pub fn graph_cell(
    row: GraphRow,
    incoming: Vec<Edge>,
    width_lanes: usize,
    colors: [Hsla; 5],
    is_merge: bool,
    marks: UnpushedMarks,
) -> impl IntoElement {
    let width = px(LANE_WIDTH * width_lanes.max(1) as f32);
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            let x_of = |lane: usize| bounds.origin.x + px(LANE_WIDTH * (lane as f32 + 0.5));
            let top = bounds.origin.y;
            let mid = top + bounds.size.height / 2.;
            let bottom = top + bounds.size.height;
            // Colours belong to branch lines (set by the layout), not to columns, so a branch
            // keeps its colour wherever it sits.
            let color = |ix: usize| colors[ix % colors.len()];

            // An edge runs from `from` at this row's middle to `to` at the next row's middle.
            // A lane change is one cubic S-curve over that whole span, with control points on
            // the row boundary above each end. Split at its midpoint (de Casteljau, t = 0.5)
            // it gives two halves that meet on the boundary with the same slope, so the curve
            // is continuous across the two canvases with no flat step.
            let h = bounds.size.height;
            let half_x = |e: &Edge| (x_of(e.from) + x_of(e.to)) / 2.;
            let edge_color = |e: &Edge| color(e.color);

            // Top half: second halves of edges arriving from the previous row.
            for (k, e) in incoming.iter().enumerate() {
                let mut b = PathBuilder::stroke(px(2.));
                if e.from == e.to {
                    b.move_to(point(x_of(e.to), top));
                    b.line_to(point(x_of(e.to), mid));
                } else {
                    // Second half: M, ((xf + 3xt) / 4, B + h/8), (xt, B + h/4), (xt, B + h/2).
                    let (xf, xt) = (x_of(e.from), x_of(e.to));
                    b.move_to(point(half_x(e), top));
                    b.cubic_bezier_to(
                        point(xt, mid),
                        point((xf + xt * 3.) / 4., top + h / 8.),
                        point(xt, top + h / 4.),
                    );
                }
                if let Ok(path) = b.build() {
                    let c = edge_color(e);
                    let marked = marks.incoming.get(k).copied().unwrap_or(false);
                    window.paint_path(path, if marked { marks.paint.apply(c) } else { c });
                }
            }
            // Bottom half: first halves of edges leaving this row.
            for (k, e) in row.edges.iter().enumerate() {
                let mut b = PathBuilder::stroke(px(2.));
                b.move_to(point(x_of(e.from), mid));
                if e.from == e.to {
                    b.line_to(point(x_of(e.to), bottom));
                } else {
                    // First half: (xf, B - h/2), (xf, B - h/4), ((3xf + xt) / 4, B - h/8), M.
                    let (xf, xt) = (x_of(e.from), x_of(e.to));
                    b.cubic_bezier_to(
                        point(half_x(e), bottom),
                        point(xf, bottom - h / 4.),
                        point((xf * 3. + xt) / 4., bottom - h / 8.),
                    );
                }
                if let Ok(path) = b.build() {
                    let c = edge_color(e);
                    let marked = marks.outgoing.get(k).copied().unwrap_or(false);
                    window.paint_path(path, if marked { marks.paint.apply(c) } else { c });
                }
            }
            // Node.
            let r = px(if is_merge {
                NODE_RADIUS + 1.5
            } else {
                NODE_RADIUS
            });
            let center = point(x_of(row.lane), mid);
            let node = Bounds::new(point(center.x - r, center.y - r), size(r * 2., r * 2.));
            let node_color = if marks.this {
                marks.paint.apply(color(row.color))
            } else {
                color(row.color)
            };
            window.paint_quad(quad(
                node,
                r,
                node_color,
                px(0.),
                Hsla::transparent_black(),
                BorderStyle::default(),
            ));
        },
    )
    .w(width)
    .h(px(ROW_HEIGHT))
    .flex_none()
}
