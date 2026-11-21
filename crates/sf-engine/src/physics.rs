//! Structural checks: overlaps, stud connections, floating groups, single-stud
//! joints and balance.

use serde::Serialize;
use sf_core::{Cell, Model};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
pub struct WeakJoint {
    /// The two parts joined by a single stud.
    pub parts: [usize; 2],
    /// Where the stud is.
    pub at: Cell,
    /// Parts that would fall off if the joint failed.
    pub parts_held: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Balance {
    pub ok: bool,
    /// Centre of mass `[x, y, z]` in grid units (studs, plates, studs).
    pub center_of_mass: [f32; 3],
    /// Distance in studs from the centre of mass to the edge of the support
    /// footprint; negative means outside.
    pub margin: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Analysis {
    /// Pairs of parts that share a cell.
    pub overlaps: Vec<[usize; 2]>,
    /// Stud connections `(a, b, studs)` with `a` below `b`.
    pub connections: Vec<(usize, usize, u32)>,
    /// Connected groups of parts; the first ones touch the ground.
    pub components: Vec<Vec<usize>>,
    /// Parts in groups that don't reach the ground.
    pub floating: Vec<usize>,
    pub weak_joints: Vec<WeakJoint>,
    pub balance: Balance,
}

struct Dsu(Vec<usize>);
impl Dsu {
    fn find(&mut self, a: usize) -> usize {
        let mut r = a;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut x = a;
        while self.0[x] != r {
            let n = self.0[x];
            self.0[x] = r;
            x = n;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

pub fn analyze(model: &Model) -> Analysis {
    let ps = &model.placements;
    let n = ps.len();
    let mut owner: HashMap<Cell, usize> = HashMap::new();
    let mut overlaps = Vec::new();
    for (i, p) in ps.iter().enumerate() {
        // Details fill only what their mesh reaches, not their whole box.
        for c in crate::details::occupied_cells(p, &model.meshes) {
            if let Some(&j) = owner.get(&c) {
                if !overlaps.contains(&[j, i]) {
                    overlaps.push([j, i]);
                }
            } else {
                owner.insert(c, i);
            }
        }
    }
    // Stud connections.
    let mut studs: HashMap<(usize, usize), (u32, Cell)> = HashMap::new();
    for (i, p) in ps.iter().enumerate() {
        let top = p.top();
        for (x, z) in p.top_studs() {
            let Some(&j) = owner.get(&Cell::new(x, top, z)) else { continue };
            let q = &ps[j];
            if j == i || q.origin.y != top || !q.bottom_sockets().contains(&(x, z)) {
                continue;
            }
            let e = studs.entry((i, j)).or_insert((0, Cell::new(x, top, z)));
            e.0 += 1;
        }
    }
    let mut connections: Vec<(usize, usize, u32)> = studs.iter().map(|(&(a, b), &(s, _))| (a, b, s)).collect();
    connections.sort();
    let grounded: Vec<bool> = ps.iter().map(|p| p.origin.y == 0).collect();

    let mut dsu = Dsu((0..n).collect());
    for &(a, b, _) in &connections {
        dsu.union(a, b);
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = dsu.find(i);
        groups.entry(r).or_default().push(i);
    }
    let mut components: Vec<Vec<usize>> = groups.into_values().collect();
    components.sort_by_key(|g| (!g.iter().any(|&i| grounded[i]), std::cmp::Reverse(g.len()), g[0]));
    let floating: Vec<usize> =
        components.iter().filter(|g| !g.iter().any(|&i| grounded[i])).flat_map(|g| g.iter().copied()).collect();

    // Adjacency for bridge finding.
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n]; // (neighbour, edge id)
    for (e, &(a, b, _)) in connections.iter().enumerate() {
        adj[a].push((b, e));
        adj[b].push((a, e));
    }
    let bridges = find_bridges(n, &adj);
    let mut weak_joints = Vec::new();
    for e in bridges {
        let (a, b, s) = connections[e];
        if s != 1 {
            continue;
        }
        // Side of the cut without the ground (or the smaller side).
        let side = |start: usize| -> Vec<usize> {
            let mut seen = vec![false; n];
            let mut stack = vec![start];
            seen[start] = true;
            let mut out = Vec::new();
            while let Some(u) = stack.pop() {
                out.push(u);
                for &(v, id) in &adj[u] {
                    if id != e && !seen[v] {
                        seen[v] = true;
                        stack.push(v);
                    }
                }
            }
            out
        };
        let sa = side(a);
        let sb = side(b);
        let ga = sa.iter().any(|&i| grounded[i]);
        let gb = sb.iter().any(|&i| grounded[i]);
        let held = match (ga, gb) {
            (true, false) => sb.len(),
            (false, true) => sa.len(),
            // Both sides stand on the ground: nothing falls if it fails.
            (true, true) => 0,
            // Already floating: reported as floating instead.
            (false, false) => 0,
        };
        if held >= 5 {
            weak_joints.push(WeakJoint { parts: [a, b], at: studs[&(a, b)].1, parts_held: held });
        }
    }
    weak_joints.sort_by_key(|w| std::cmp::Reverse(w.parts_held));

    Analysis { overlaps, connections, components, floating, weak_joints, balance: balance(model) }
}

/// Tarjan's bridge-finding (iterative).
fn find_bridges(n: usize, adj: &[Vec<(usize, usize)>]) -> Vec<usize> {
    let mut disc = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut t = 0;
    let mut out = Vec::new();
    for s in 0..n {
        if disc[s] != usize::MAX {
            continue;
        }
        // (node, parent edge, next adjacency index)
        let mut stack: Vec<(usize, usize, usize)> = vec![(s, usize::MAX, 0)];
        disc[s] = t;
        low[s] = t;
        t += 1;
        while let Some(&mut (u, pe, ref mut i)) = stack.last_mut() {
            if *i < adj[u].len() {
                let (v, e) = adj[u][*i];
                *i += 1;
                if e == pe {
                    continue;
                }
                if disc[v] == usize::MAX {
                    disc[v] = t;
                    low[v] = t;
                    t += 1;
                    stack.push((v, e, 0));
                } else {
                    low[u] = low[u].min(disc[v]);
                }
            } else {
                stack.pop();
                if let Some(&(p, _, _)) = stack.last() {
                    low[p] = low[p].min(low[u]);
                    if low[u] > disc[p] {
                        out.push(pe);
                    }
                }
            }
        }
    }
    out
}

fn balance(model: &Model) -> Balance {
    let mut m = 0.0f32;
    let mut c = [0.0f32; 3];
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for p in &model.placements {
        let [fx, fz] = p.footprint();
        let h = p.height() as f32;
        let w = (fx * fz) as f32 * h * p.fill_fraction();
        c[0] += w * (p.origin.x as f32 + fx as f32 / 2.0);
        c[1] += w * (p.origin.y as f32 + h / 2.0);
        c[2] += w * (p.origin.z as f32 + fz as f32 / 2.0);
        m += w;
        if p.origin.y == 0 {
            for (x, z) in p.bottom_sockets() {
                let (x, z) = (x as f32, z as f32);
                pts.extend([(x, z), (x + 1.0, z), (x, z + 1.0), (x + 1.0, z + 1.0)]);
            }
        }
    }
    if m == 0.0 || pts.is_empty() {
        return Balance { ok: pts.is_empty() && m == 0.0, center_of_mass: [0.0; 3], margin: 0.0 };
    }
    let com = [c[0] / m, c[1] / m, c[2] / m];
    let hull = convex_hull(pts);
    let margin = signed_distance(&hull, (com[0], com[2]));
    Balance { ok: margin >= 0.25, center_of_mass: com, margin }
}

pub(crate) fn convex_hull(mut pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f32, f32)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f32, f32)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Positive inside a counter-clockwise convex polygon, negative outside.
pub(crate) fn signed_distance(hull: &[(f32, f32)], p: (f32, f32)) -> f32 {
    if hull.len() < 3 {
        return -1.0;
    }
    let mut d = f32::MAX;
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let len = (ex * ex + ey * ey).sqrt();
        // For CCW order, inside is to the left of each edge.
        let s = (ex * (p.1 - a.1) - ey * (p.0 - a.0)) / len;
        d = d.min(s);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::{Placement, Rot};

    fn m(ps: Vec<Placement>) -> Model {
        Model { placements: ps, ..Default::default() }
    }

    #[test]
    fn stacked_bricks_connect() {
        let a = analyze(&m(vec![
            Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0)),
            Placement::new("3001", 4, Cell::new(2, 3, 0), Rot(0)),
        ]));
        assert_eq!(a.connections, vec![(0, 1, 4)]);
        assert!(a.floating.is_empty());
        assert_eq!(a.components.len(), 1);
    }

    #[test]
    fn floating_and_weak() {
        let a = analyze(&m(vec![
            Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0)),
            // joined by one stud at (3, 1); holds 4 parts
            Placement::new("3001", 4, Cell::new(3, 3, 1), Rot(0)),
            Placement::new("3001", 4, Cell::new(4, 6, 1), Rot(0)),
            Placement::new("3001", 4, Cell::new(4, 9, 1), Rot(0)),
            Placement::new("3001", 4, Cell::new(4, 12, 1), Rot(0)),
            Placement::new("3001", 4, Cell::new(4, 15, 1), Rot(0)),
            // floating
            Placement::new("3005", 4, Cell::new(20, 5, 20), Rot(0)),
        ]));
        assert_eq!(a.floating, vec![6]);
        assert_eq!(a.weak_joints.len(), 1);
        assert_eq!(a.weak_joints[0].parts_held, 5);
    }

    #[test]
    fn overhang_is_off_balance() {
        let mut ps = vec![Placement::new("3005", 4, Cell::new(0, 0, 0), Rot(0))];
        ps.push(Placement::new("3035", 4, Cell::new(0, 3, 0), Rot(0)));
        for i in 0..4 {
            ps.push(Placement::new("3035", 4, Cell::new(0, 4 + i, 0), Rot(0)));
        }
        let a = analyze(&m(ps));
        assert!(!a.balance.ok, "{:?}", a.balance);
        let b = analyze(&m(vec![Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0))]));
        assert!(b.balance.ok, "{:?}", b.balance);
    }

    #[test]
    fn overlap_detected() {
        let a = analyze(&m(vec![
            Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0)),
            Placement::new("3003", 4, Cell::new(1, 1, 0), Rot(0)),
        ]));
        assert_eq!(a.overlaps, vec![[0, 1]]);
    }
}
