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
}

impl UnpushedPaint {
    fn apply(self, color: Hsla) -> Hsla {
        match self {
            UnpushedPaint::Dim(background) => background.blend(color.opacity(0.4)),
            UnpushedPaint::Colour(c) => c,
        }
    }
}

/// Which parts of a row belong to unpushed commits: this row's node and its links to its
/// parents (`this`), and the links arriving from the previous row's commit (`prev`: that
/// row's lane, when it is unpushed). Lanes that only pass through are left alone.
#[derive(Debug, Clone, Copy)]
pub struct UnpushedMarks {
    pub paint: UnpushedPaint,
    pub this: bool,
    pub prev: Option<usize>,
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
            let color = |lane: usize| colors[lane % colors.len()];

            // An edge runs from `from` at this row's middle to `to` at the next row's middle.
            // A lane change is one cubic S-curve over that whole span, with control points on
            // the row boundary above each end. Split at its midpoint (de Casteljau, t = 0.5)
            // it gives two halves that meet on the boundary with the same slope, so the curve
            // is continuous across the two canvases with no flat step.
            let h = bounds.size.height;
            let half_x = |e: &Edge| (x_of(e.from) + x_of(e.to)) / 2.;
            // A sideways edge belongs to the side branch (the outer lane), whether it forks off
            // here or merges in, so both halves take that lane's colour.
            let edge_color = |e: &Edge| {
                if e.from == e.to {
                    color(e.to)
                } else {
                    color(e.from.max(e.to))
                }
            };

            // Top half: second halves of edges arriving from the previous row.
            for e in &incoming {
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
                    let marked = marks.prev == Some(e.from);
                    window.paint_path(path, if marked { marks.paint.apply(c) } else { c });
                }
            }
            // Bottom half: first halves of edges leaving this row.
            for e in &row.edges {
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
                    let marked = marks.this && e.from == row.lane;
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
                marks.paint.apply(color(row.lane))
            } else {
                color(row.lane)
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
