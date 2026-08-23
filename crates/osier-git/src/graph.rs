//! Lane layout for the commit graph.
//!
//! Pure function over a list of commits in log order (children before parents). The UI draws
//! each [`GraphRow`]: the node sits in `lane`, and `edges` are the line segments from this
//! row to the next one. Lanes are reused as soon as they end, keeping the graph narrow.

use crate::log::Commit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub hash: String,
    /// Column of the commit node.
    pub lane: usize,
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
}

/// Compute the layout. Lanes are assigned by first appearance; colour by lane in the UI.
pub fn layout(commits: &[Commit]) -> Vec<GraphRow> {
    // Each active lane holds the hash of the commit expected to appear next in it.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        // Lane of this commit: the first lane waiting for it, otherwise a fresh one.
        let lane = match lanes
            .iter()
            .position(|l| l.as_deref() == Some(&commit.hash))
        {
            Some(i) => i,
            None => {
                let i = first_free(&lanes);
                set(&mut lanes, i, Some(commit.hash.clone()));
                i
            }
        };
        // Other lanes also waiting for this commit (merges seen from several children) collapse
        // into `lane`.
        let merged_into: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(i, l)| *i != lane && l.as_deref() == Some(&commit.hash))
            .map(|(i, _)| i)
            .collect();
        for &i in &merged_into {
            lanes[i] = None;
        }

        // Next row: the commit's lane continues with its first parent, extra parents get their
        // own lanes (or join an existing lane already waiting for that parent).
        let mut edges = Vec::new();
        let mut next = lanes.clone();
        next[lane] = None;
        for (pi, parent) in commit.parents.iter().enumerate() {
            let target = match next.iter().position(|l| l.as_deref() == Some(parent)) {
                // The first parent takes over this lane when it was waiting further right, so
                // the main line stays in the lowest column.
                Some(j) if pi == 0 && j > lane => {
                    next[j] = None;
                    next[lane] = Some(parent.clone());
                    lane
                }
                Some(j) => j,
                None => {
                    // Parents outside the loaded window still get a lane so the line runs off
                    // the page.
                    let i = if pi == 0 { lane } else { first_free(&next) };
                    set(&mut next, i, Some(parent.clone()));
                    i
                }
            };
            edges.push(Edge {
                from: lane,
                to: target,
                parent: Some(pi),
            });
        }
        // Lanes that merged into this node draw a line from their column to the node.
        for &i in &merged_into {
            edges.insert(
                0,
                Edge {
                    from: i,
                    to: lane,
                    parent: None,
                },
            );
        }
        // Every other active lane continues to wherever its awaited commit now sits.
        for (i, l) in lanes.iter().enumerate() {
            if i == lane || merged_into.contains(&i) {
                continue;
            }
            if let Some(h) = l
                && let Some(to) = next.iter().position(|n| n.as_deref() == Some(h))
            {
                edges.push(Edge {
                    from: i,
                    to,
                    parent: None,
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
            edges,
            width,
        });
        lanes = next;
    }
    rows
}

fn first_free(lanes: &[Option<String>]) -> usize {
    lanes
        .iter()
        .position(Option::is_none)
        .unwrap_or(lanes.len())
}

fn set(lanes: &mut Vec<Option<String>>, i: usize, v: Option<String>) {
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
            parent: Some(1)
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

    #[test]
    fn parent_outside_window_keeps_a_lane_open() {
        let rows = layout(&[c("b", &["a"]), c("c", &["zz"])]);
        assert_eq!(rows[1].width, 2, "both lanes continue off the page");
    }
}
