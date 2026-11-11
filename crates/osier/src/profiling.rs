//! Developer-only frame profiling on top of gpui's profiler. Compiled only with the
//! `profiler` feature (`cargo run -p osier --features profiler`); normal builds have none of it.
//!
//! - `OSIER_FRAME_OVERLAY=minimal|full`: gpui's frame-time overlay, painted into the window.
//!   Minimal shows the last frame's draw time; full adds the 1% and 10% lows and the maximum.
//! - `OSIER_PROFILE=1`, or a number of milliseconds (default 8.3, one frame at 120Hz): log each
//!   frame or piece of main-thread work that took longer, with where the time went (draw
//!   phases, the slowest views, presenting, actions, tasks), and a frame-time summary when
//!   Osier quits. Logged at info level under the `osier::profiling` target.
//!
//! Draw times are main-thread CPU time; GPU work shows up indirectly, as "dirty to present"
//! and as the gap between animation frames in the summary.

use std::collections::HashMap;
use std::time::Duration;

use gpui::profiler::hang::{HangIncident, HangMonitorConfig};
use gpui::profiler::journal::ForegroundEvent;
use gpui::{DebugFrameOverlayMode, ViewPhase};
use gpui_kit::{AnyWindowHandle, App};

const TARGET: &str = "osier::profiling";
/// One frame at 120Hz.
const DEFAULT_THRESHOLD: Duration = Duration::from_nanos(8_333_333);

/// Turn on what the environment asks for, for the main window.
pub(crate) fn start(window: AnyWindowHandle, cx: &mut App) {
    if let Some(mode) = overlay_mode(std::env::var("OSIER_FRAME_OVERLAY").ok().as_deref()) {
        let _ = window.update(cx, |_, window, _| window.set_debug_frame_overlay_mode(mode));
    }
    let Some(threshold) = threshold(std::env::var("OSIER_PROFILE").ok().as_deref()) else {
        return;
    };
    tracing::info!(
        target: TARGET,
        "profiling: logging work over {:.1} ms",
        threshold.as_secs_f64() * 1000.
    );
    let config = HangMonitorConfig {
        threshold,
        frame_budget: threshold,
        interval: Duration::from_millis(250),
    };
    let started = cx.start_hang_monitor(config, |poll| {
        for incident in &poll.incidents {
            tracing::info!(target: TARGET, "{}", describe(incident));
        }
    });
    if let Err(error) = started {
        tracing::warn!(target: TARGET, %error, "could not start the frame monitor");
    }
    cx.on_app_quit(move |cx| {
        let _ = window.update(cx, |_, window, _| {
            tracing::info!(target: TARGET, "{}", summary(&window.frame_duration_snapshot()));
        });
        async {}
    })
    .detach();
}

fn overlay_mode(value: Option<&str>) -> Option<DebugFrameOverlayMode> {
    match value?.trim() {
        "minimal" | "1" => Some(DebugFrameOverlayMode::Minimal),
        "full" => Some(DebugFrameOverlayMode::Full),
        _ => None,
    }
}

/// `1` (or anything that is not a positive number) means the default threshold.
fn threshold(value: Option<&str>) -> Option<Duration> {
    let value = value?.trim();
    if value.is_empty() || value == "0" {
        return None;
    }
    Some(
        value
            .parse::<f64>()
            .ok()
            .filter(|ms| *ms > 1.)
            .map_or(DEFAULT_THRESHOLD, |ms| Duration::from_secs_f64(ms / 1000.)),
    )
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.)
}

/// One line per slow interval: what triggered it, and where the main thread spent the time.
fn describe(incident: &HangIncident) -> String {
    let snapshot = &incident.snapshot;
    let mut parts = vec![format!(
        "slow ({}): busy {}",
        incident.trigger.as_str(),
        ms(snapshot.occupancy())
    )];
    let mut views: HashMap<(&'static str, &'static str), Duration> = HashMap::new();
    let mut present = Duration::ZERO;
    for event in &snapshot.events {
        match event {
            ForegroundEvent::Draw(frame) => parts.push(format!(
                "draw {} (layout {}, prepaint {}, paint {}, {} invalidations)",
                ms(event.duration()),
                ms(frame.phases.request_layout),
                ms(frame.phases.prepaint),
                ms(frame.phases.paint),
                frame.invalidations
            )),
            ForegroundEvent::View(view) => {
                let name = view.name.rsplit("::").next().unwrap_or(view.name);
                let phase = match view.phase {
                    ViewPhase::RequestLayout => "layout",
                    ViewPhase::Prepaint => "prepaint",
                    ViewPhase::Paint => "paint",
                };
                *views.entry((name, phase)).or_default() += view.self_duration;
            }
            ForegroundEvent::Present(_) => present += event.duration(),
            ForegroundEvent::Action(action) => {
                parts.push(format!("action {} {}", action.name, ms(event.duration())))
            }
            ForegroundEvent::Input(input) => {
                parts.push(format!("input {} {}", input.kind, ms(event.duration())))
            }
            ForegroundEvent::TaskPoll(task) => parts.push(format!(
                "task {}:{} {}",
                task.location.file(),
                task.location.line(),
                ms(event.duration())
            )),
            _ => {}
        }
    }
    let mut views: Vec<_> = views.into_iter().collect();
    views.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    if !views.is_empty() {
        let top: Vec<String> = views
            .iter()
            .take(5)
            .map(|((name, phase), d)| format!("{name} {phase} {}", ms(*d)))
            .collect();
        parts.push(format!("views (own time): {}", top.join(", ")));
    }
    if present > Duration::ZERO {
        parts.push(format!("present {}", ms(present)));
    }
    parts.join("; ")
}

/// Percentiles of the frame histograms gpui kept for the window. A macro, not a function, so
/// Osier does not have to name (and depend on) the histogram crate gpui uses.
macro_rules! histogram_line {
    ($label:expr, $h:expr) => {{
        let h = $h;
        if h.is_empty() {
            format!("{}: none", $label)
        } else {
            let at = |q: f64| ms(Duration::from_nanos(h.value_at_quantile(q)));
            format!(
                "{}: {} frames, median {}, p90 {}, p99 {}, max {}",
                $label,
                h.len(),
                at(0.5),
                at(0.9),
                at(0.99),
                ms(Duration::from_nanos(h.max()))
            )
        }
    }};
}

fn summary(s: &gpui::FrameDurationSnapshot) -> String {
    format!(
        "frame summary - {}; {}; {}",
        histogram_line!("draw (CPU)", &s.draw_duration_histogram),
        histogram_line!("change to screen", &s.dirty_to_present_histogram),
        histogram_line!("between animation frames", &s.present_interval_histogram),
    )
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_THRESHOLD, overlay_mode, threshold};
    use gpui::DebugFrameOverlayMode;
    use std::time::Duration;

    #[test]
    fn environment_values() {
        assert_eq!(
            overlay_mode(Some("full")),
            Some(DebugFrameOverlayMode::Full)
        );
        assert_eq!(
            overlay_mode(Some("minimal")),
            Some(DebugFrameOverlayMode::Minimal)
        );
        assert_eq!(overlay_mode(Some("nope")), None);
        assert_eq!(overlay_mode(None), None);
        assert_eq!(threshold(None), None);
        assert_eq!(threshold(Some("0")), None);
        assert_eq!(threshold(Some("1")), Some(DEFAULT_THRESHOLD));
        assert_eq!(threshold(Some("16")), Some(Duration::from_millis(16)));
    }
}
