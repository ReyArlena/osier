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

/// Canvas element for a row: incoming edges from the previous row, outgoing edges, node.
pub fn graph_cell(
    row: GraphRow,
    incoming: Vec<Edge>,
    width_lanes: usize,
    colors: [Hsla; 5],
    is_merge: bool,
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

            // Top half: edges arriving from the previous row. The previous row's bottom half
            // already moved them into their `to` lane, so they continue straight down.
            for e in &incoming {
                let mut b = PathBuilder::stroke(px(2.));
                b.move_to(point(x_of(e.to), top));
                b.line_to(point(x_of(e.to), mid));
                if let Ok(path) = b.build() {
                    window.paint_path(path, color(e.to));
                }
            }
            // Bottom half: edges leaving this row.
            for e in &row.edges {
                let mut b = PathBuilder::stroke(px(2.));
                b.move_to(point(x_of(e.from), mid));
                if e.from == e.to {
                    b.line_to(point(x_of(e.to), bottom));
                } else {
                    b.curve_to(point(x_of(e.to), bottom), point(x_of(e.to), mid));
                }
                // A sideways edge belongs to the side branch (the outer lane), whether it
                // forks off here or merges in, so it takes that lane's colour.
                let lane = if e.from == e.to { e.to } else { e.from.max(e.to) };
                if let Ok(path) = b.build() {
                    window.paint_path(path, color(lane));
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
            window.paint_quad(quad(
                node,
                r,
                color(row.lane),
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
