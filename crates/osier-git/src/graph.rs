//! Lane layout for the commit graph.
//!
//! Pure function over a list of commits in log order (children before parents). The UI draws
//! each [`GraphRow`]: the node sits in `lane`, and `edges` are the line segments from this
//! row to the next one. A branch keeps its column and its colour from its tip down to the
//! commit it forked from, and its line joins that commit's node there, like PhpStorm draws
//! it. Columns are reused once their branch has ended.

use crate::log::Commit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub hash: String,
    /// Column of the commit node.
    pub lane: usize,
    /// Colour of the node: the colour of the branch line it sits on. An index the UI maps
    /// into its palette (cycling).
    pub color: usize,
    /// Segments leaving this row: (from lane in this row, to lane in the next row).
    pub edges: Vec<Edge>,
    /// Number of lanes occupied below this row (width hint).
    pub width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// Parent index on the node that created the edge (0 = first parent) when the edge starts
    /// at this row's node, `None` for pass-through lanes.
    pub parent: Option<usize>,
    /// Colour of the branch line this segment belongs to (see [`GraphRow::color`]).
    pub color: usize,
}

/// Compute the layout.
pub fn layout(commits: &[Commit]) -> Vec<GraphRow> {
    // Each active lane holds the hash of the commit expected to appear next in it and the
    // colour of its line. Several lanes may wait for the same commit (branches forked from
    // it); they meet at its node.
    let mut lanes: Vec<Option<(String, usize)>> = Vec::new();
    let mut next_color = 0;
    let mut new_color = || {
        let c = next_color;
        next_color += 1;
        c
    };
    let mut rows = Vec::with_capacity(commits.len());

    for (ix, commit) in commits.iter().enumerate() {
        let waits_for =
            |l: &Option<(String, usize)>, h: &str| l.as_ref().is_some_and(|(x, _)| x == h);
        // Lane of this commit: the leftmost lane waiting for it, otherwise a fresh one (a
        // branch tip).
        let (lane, color) = match lanes.iter().position(|l| waits_for(l, &commit.hash)) {
            Some(i) => (i, lanes[i].as_ref().map_or(0, |(_, c)| *c)),
            None => {
                let i = first_free(&lanes);
                let c = new_color();
                set(&mut lanes, i, Some((commit.hash.clone(), c)));
                (i, c)
            }
        };
        // Every lane waiting for this commit ends here: their lines arrived at the node from
        // the row above (see `dest`).
        let mut next = lanes.clone();
        for l in next.iter_mut() {
            if waits_for(l, &commit.hash) {
                *l = None;
            }
        }

        // The node's own line continues straight down to its first parent, even when other
        // lanes wait for that commit too: they meet at the parent's node, not here. Extra
        // parents (merges) join a lane already waiting for them, or open one in a new
        // colour.
        let mut parent_lanes = Vec::with_capacity(commit.parents.len());
        for (pi, parent) in commit.parents.iter().enumerate() {
            if pi == 0 {
                set(&mut next, lane, Some((parent.clone(), color)));
                parent_lanes.push((lane, color));
            } else if let Some(j) = next.iter().position(|l| waits_for(l, parent)) {
                parent_lanes.push((j, next[j].as_ref().map_or(color, |(_, c)| *c)));
            } else {
                let i = first_free(&next);
                let c = new_color();
                set(&mut next, i, Some((parent.clone(), c)));
                parent_lanes.push((i, c));
            }
        }

        // Where a line in lane `i` waiting for `hash` arrives in the next row: straight down,
        // unless the next row is that commit, which sits in the leftmost lane waiting for it.
        let following = commits.get(ix + 1).map(|c| c.hash.as_str());
        let dest = |i: usize, hash: &str| {
            if following == Some(hash) {
                next.iter().position(|l| waits_for(l, hash)).unwrap_or(i)
            } else {
                i
            }
        };

        let mut edges = Vec::new();
        for (pi, (parent, &(target, c))) in commit.parents.iter().zip(&parent_lanes).enumerate() {
            edges.push(Edge {
                from: lane,
                to: dest(target, parent),
                parent: Some(pi),
                // The first parent continues this line; a merge line is the merged branch's.
                color: c,
            });
        }
        // Every other active lane passes through.
        for (i, l) in lanes.iter().enumerate() {
            if let Some((h, c)) = l
                && h != &commit.hash
            {
                edges.push(Edge {
                    from: i,
                    to: dest(i, h),
                    parent: None,
                    color: *c,
                });
            }
        }
        // Trim trailing empty lanes.
        while matches!(next.last(), Some(None)) {
            next.pop();
        }
        let width = next.len().max(lane + 1);
        rows.push(GraphRow {
            hash: commit.hash.clone(),
            lane,
            color,
            edges,
            width,
        });
        lanes = next;
    }
    rows
}

fn first_free<T>(lanes: &[Option<T>]) -> usize {
    lanes
        .iter()
        .position(Option::is_none)
        .unwrap_or(lanes.len())
}

fn set<T: Clone>(lanes: &mut Vec<Option<T>>, i: usize, v: Option<T>) {
    if i >= lanes.len() {
        lanes.resize(i + 1, None);
    }
    lanes[i] = v;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            committed_at: 0,
            refs: vec![],
            subject: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn linear_history_is_one_lane() {
        let rows = layout(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(rows.iter().all(|r| r.width == 1));
        assert_eq!(rows[2].edges, vec![]);
    }

    #[test]
    fn merge_opens_second_lane_and_closes_it() {
        // m merges f into main: m -> (b, f), f -> a, b -> a, a
        let rows = layout(&[
            c("m", &["b", "f"]),
            c("f", &["a"]),
            c("b", &["a"]),
            c("a", &[]),
        ]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].width, 2);
        assert!(rows[0].edges.contains(&Edge {
            from: 0,
            to: 1,
            parent: Some(1),
            color: 1,
        }));
        assert_eq!(rows[1].lane, 1, "f lives in lane 1");
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[3].width, 1, "lanes collapse at the common ancestor");
    }

    #[test]
    fn unrelated_branch_tip_gets_its_own_lane() {
        // two heads: x (lane 0) and y (lane 1), both on top of a
        let rows = layout(&[c("x", &["a"]), c("y", &["a"]), c("a", &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        assert!(rows[1].edges.iter().any(|e| e.from == 1 && e.to == 0));
    }

    /// Each row as (node lane, [(from, to)]) for compact assertions.
    fn shape(rows: &[GraphRow]) -> Vec<(usize, Vec<(usize, usize)>)> {
        rows.iter()
            .map(|r| {
                let mut e: Vec<_> = r.edges.iter().map(|e| (e.from, e.to)).collect();
                e.sort();
                (r.lane, e)
            })
            .collect()
    }

    /// Three branches forked from `b`, in date order (the cachyup history from the bug
    /// report): `x` on its own, the stow branch `s1..s3`, main `m1..m2`. Each keeps its
    /// column and colour down to `b`, and both side lines join `b`'s node.
    #[test]
    fn branches_keep_their_lane_and_colour_down_to_the_fork() {
        let commits = [
            c("x", &["b"]),
            c("s3", &["s2"]),
            c("m2", &["m1"]),
            c("m1", &["b"]),
            c("s2", &["s1"]),
            c("s1", &["b"]),
            c("b", &[]),
        ];
        let rows = layout(&commits);
        check_continuity(&commits, &rows);
        assert_eq!(
            shape(&rows),
            vec![
                (0, vec![(0, 0)]),
                (1, vec![(0, 0), (1, 1)]),
                (2, vec![(0, 0), (1, 1), (2, 2)]),
                (2, vec![(0, 0), (1, 1), (2, 2)]),
                (1, vec![(0, 0), (1, 1), (2, 2)]),
                (1, vec![(0, 0), (1, 0), (2, 0)]),
                (0, vec![]),
            ]
        );
        let colors: Vec<usize> = rows.iter().map(|r| r.color).collect();
        assert_eq!(colors, vec![0, 1, 2, 2, 1, 1, 0]);
        // The lines joining `b` keep their branch's colour.
        let joins: Vec<(usize, usize)> = rows[5]
            .edges
            .iter()
            .filter(|e| e.to == 0)
            .map(|e| (e.from, e.color))
            .collect();
        assert!(joins.contains(&(1, 1)) && joins.contains(&(2, 2)) && joins.contains(&(0, 0)));
        assert_eq!(rows[6].width, 1);
    }

    #[test]
    fn a_column_is_reused_once_its_branch_has_ended() {
        // y forks from a and ends there; z, a later tip above a's parent, takes its column.
        let commits = [
            c("x", &["a"]),
            c("y", &["a"]),
            c("a", &["r"]),
            c("z", &["r"]),
            c("r", &[]),
        ];
        let rows = layout(&commits);
        check_continuity(&commits, &rows);
        assert_eq!(rows[3].lane, 1);
        assert_ne!(
            rows[3].color, rows[1].color,
            "a new branch gets a new colour"
        );
    }

    #[test]
    fn merge_line_takes_the_merged_branch_colour() {
        // t and m both reach f; m merges it: the merge line is drawn in f's line colour.
        let commits = [
            c("t", &["f"]),
            c("m", &["b", "f"]),
            c("f", &["b"]),
            c("b", &[]),
        ];
        let rows = layout(&commits);
        check_continuity(&commits, &rows);
        let merge = rows[1].edges.iter().find(|e| e.parent == Some(1)).unwrap();
        assert_eq!((merge.from, merge.to, merge.color), (1, 0, rows[0].color));
        assert_eq!(rows[2].lane, 0);
    }

    #[test]
    fn continuity_holds_for_every_shape_above() {
        let cases: Vec<Vec<Commit>> = vec![
            vec![
                c("m", &["b", "f"]),
                c("f", &["a"]),
                c("b", &["a"]),
                c("a", &[]),
            ],
            vec![c("x", &["a"]), c("y", &["a"]), c("a", &[])],
            // Octopus merge and a parent outside the page.
            vec![
                c("o", &["a", "b", "c"]),
                c("c", &["a"]),
                c("b", &["zz"]),
                c("a", &[]),
            ],
            // Topo order of the bug report history: stow's commits before main's.
            vec![
                c("x", &["b"]),
                c("s3", &["s2"]),
                c("s2", &["s1"]),
                c("s1", &["b"]),
                c("m2", &["m1"]),
                c("m1", &["b"]),
                c("b", &[]),
            ],
        ];
        for commits in cases {
            check_continuity(&commits, &layout(&commits));
        }
    }

    /// Every line that leaves a row arrives somewhere in the next one (at its node or on a
    /// line that keeps going), and every line in a row came from the row above or starts at
    /// the row's own node.
    fn check_continuity(commits: &[Commit], rows: &[GraphRow]) {
        for (r, pair) in rows.windows(2).enumerate() {
            let (above, below) = (&pair[0], &pair[1]);
            for e in &above.edges {
                assert!(
                    e.to == below.lane || below.edges.iter().any(|b| b.from == e.to),
                    "row {r} ({}): edge {e:?} ends in lane {} where nothing continues",
                    commits[r].hash,
                    e.to
                );
            }
            for b in &below.edges {
                assert!(
                    b.from == below.lane || above.edges.iter().any(|e| e.to == b.from),
                    "row {} ({}): edge {b:?} starts from nowhere",
                    r + 1,
                    commits[r + 1].hash
                );
            }
        }
    }

    /// Stats and continuity on real repositories (not run by default):
    /// `OSIER_GRAPH_REPOS=/a:/b OSIER_GRAPH_ORDER=date cargo test -p osier-git graph_on_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn graph_on_real_repos() {
        let repos = std::env::var("OSIER_GRAPH_REPOS").unwrap_or_default();
        let order = std::env::var("OSIER_GRAPH_ORDER").unwrap_or_else(|_| "topo".into());
        for repo in repos.split(':').filter(|r| !r.is_empty()) {
            let out = std::process::Command::new("git")
                .args(["-C", repo, "log", "--all", "-n2000", "--format=%H %P"])
                .arg(format!("--{order}-order"))
                .output()
                .unwrap();
            let commits: Vec<Commit> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| {
                    let mut it = l.split(' ');
                    let hash = it.next().unwrap();
                    c(hash, &it.collect::<Vec<_>>())
                })
                .collect();
            let start = std::time::Instant::now();
            let rows = layout(&commits);
            let took = start.elapsed();
            check_continuity(&commits, &rows);
            let max = rows.iter().map(|r| r.width).max().unwrap_or(0);
            let mean =
                rows.iter().map(|r| r.width).sum::<usize>() as f64 / rows.len().max(1) as f64;
            println!(
                "{order} {repo}: {} rows, max width {max}, mean {mean:.2}, {took:?}",
                rows.len()
            );
        }
    }

    #[test]
    fn parent_outside_window_keeps_a_lane_open() {
        let rows = layout(&[c("b", &["a"]), c("c", &["zz"])]);
        assert_eq!(rows[1].width, 2, "both lanes continue off the page");
    }
}
