//! Build step ordering.
//!
//! Parts go in bottom-up. A part is added only once it can attach: it sits on
//! the ground or connects to a part already placed (so parts hanging under
//! an overhang wait for the part above them). Each step holds parts of one
//! layer, up to `MAX_PER_STEP`, in reading order.

use crate::physics::Analysis;
use sf_core::Model;

pub const MAX_PER_STEP: usize = 10;

pub fn order_steps(model: &Model, analysis: &Analysis) -> Vec<Vec<usize>> {
    let ps = &model.placements;
    let n = ps.len();
    let mut nbrs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b, _) in &analysis.connections {
        nbrs[a].push(b);
        nbrs[b].push(a);
    }
    let mut placed = vec![false; n];
    let mut steps: Vec<Vec<usize>> = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        let ready: Vec<usize> =
            (0..n).filter(|&i| !placed[i] && (ps[i].origin.y == 0 || nbrs[i].iter().any(|&j| placed[j]))).collect();
        let Some(level) = ready.iter().map(|&i| ps[i].origin.y).min() else {
            // Floating parts: add the rest in one final step.
            let rest: Vec<usize> = (0..n).filter(|&i| !placed[i]).collect();
            steps.push(rest);
            break;
        };
        // Grow the set at this level until nothing more attaches.
        let mut batch: Vec<usize> = Vec::new();
        let mut frontier: Vec<usize> = ready.into_iter().filter(|&i| ps[i].origin.y == level).collect();
        while !frontier.is_empty() {
            for &i in &frontier {
                placed[i] = true;
            }
            batch.extend(&frontier);
            frontier = (0..n)
                .filter(|&i| !placed[i] && ps[i].origin.y == level && nbrs[i].iter().any(|&j| placed[j]))
                .collect();
        }
        remaining -= batch.len();
        batch.sort_by_key(|&i| (ps[i].origin.z, ps[i].origin.x));
        steps.extend(batch.chunks(MAX_PER_STEP).map(|c| c.to_vec()));
    }
    steps
}
