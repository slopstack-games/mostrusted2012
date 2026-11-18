//! The driver's path: the game's ring of 0x50-byte path nodes at driver +0x8a0 (`FUN_004e7d80` builds a race path,
//! resampled every 15 m by `FUN_004e95e0`), and the path queries the racer state calls.
//!
//! Node layout (the fields kept): +0x00 / +0x04 the line point (x, z), +0x08 / +0x0c the Hermite tangent,
//! +0x28 the turn, +0x38 / +0x3c the side clearances, +0x4c flags. The driver's current node (+0x8a8) is the
//! start of the segment the car is on; the ring holds at most [`RING`] nodes ahead.

use crate::{AiConfig, V2, wrap_angle};

/// Nodes the game keeps ahead of the car (the 64-node ring is filled with at most 48, `FUN_004e95e0`).
pub const RING: usize = 48;
/// Side clearance of a node with nothing within the nav raycast's 100 m, less its 0.5 m margin (`FUN_004e58e0`).
pub const OPEN_CLEARANCE: f32 = 99.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathNode {
    /// +0x00 / +0x04: the point the car follows (the game's relaxed racing line; ours: whatever the runtime hands
    /// in).
    pub position: V2,
    /// +0x08 / +0x0c: the Hermite tangent (`FUN_004e6160`).
    pub tangent: V2,
    /// +0x28: wrap(angle of the next segment − angle of the previous one), angles `atan2(z, x)`
    /// (`FUN_004e6a10`): positive turns to the right; 0 at the path's ends.
    pub turn: f32,
    /// +0x38 / +0x3c: side clearances (m, nav raycasts along ± the side vector, `FUN_004e58e0`). The look-ahead
    /// reads +0x38 when the summed turn is positive (a right-hand corner), so +0x38 is inferred to be the right side.
    pub clearance: [f32; 2],
    /// +0x4c: flags (0x80 marks a corner node for the look-ahead, 0x10 counts in `FUN_00512b70`).
    pub flags: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub nodes: Vec<PathNode>,
    /// A circuit: the last node joins the first.
    pub closed: bool,
}

impl Path {
    /// A path through `points` with the game's tangents (`FUN_004e6160`) and turns (`FUN_004e6a10`); clearances
    /// open ([`OPEN_CLEARANCE`]) until the runtime fills them.
    #[allow(clippy::needless_range_loop)]
    pub fn new(points: &[V2], closed: bool) -> Self {
        let n = points.len();
        let mut nodes: Vec<PathNode> = points
            .iter()
            .map(|&position| PathNode {
                position,
                tangent: V2::default(),
                turn: 0.0,
                clearance: [OPEN_CLEARANCE; 2],
                flags: 0,
            })
            .collect();
        if n >= 2 {
            let at = |k: isize| points[k.rem_euclid(n as isize) as usize];
            for k in 0..n {
                let ki = k as isize;
                let interior = closed || (k > 0 && k + 1 < n);
                nodes[k].tangent = if interior {
                    // The bisector of the unit segment directions, with length (|prev| + |next|) / 2; a reversal
                    // (bisector 0) takes the previous direction turned by 90° (−y, x).
                    let (a, b) = (at(ki) - at(ki - 1), at(ki + 1) - at(ki));
                    let (la, lb) = (a.length(), b.length());
                    let (ua, ub) = (a.normalize(), b.normalize());
                    let mut d = ua + ub;
                    if d.length() <= f32::EPSILON {
                        d = V2::new(-ua.z, ua.x);
                    }
                    d.normalize() * ((la + lb) * 0.5)
                } else if k == 0 {
                    // First node: 0.5 × (next − this).
                    (at(1) - at(0)) * 0.5
                } else {
                    // Last node of an open path: not written by `FUN_004e6160` (it keeps an older value). Model
                    // decision: 0.5 × (this − previous).
                    (at(ki) - at(ki - 1)) * 0.5
                };
                if interior {
                    let (a, b) = (at(ki) - at(ki - 1), at(ki + 1) - at(ki));
                    nodes[k].turn = wrap_angle(b.normalize().angle() - a.normalize().angle());
                }
            }
        }
        Self { nodes, closed }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Nodes in the ring from `cur` on (the game's ring count, +0x8ac).
    pub fn count_from(&self, cur: usize) -> usize {
        let left = if self.closed { self.nodes.len() } else { self.nodes.len().saturating_sub(cur) };
        left.min(RING)
    }

    /// The node `j` places after `cur`. The path must not be empty (callers check `count_from(cur) >= 2`).
    pub fn node(&self, cur: usize, j: usize) -> &PathNode {
        &self.nodes[(cur + j) % self.nodes.len()]
    }

    fn segment_length(&self, cur: usize, j: usize) -> f32 {
        (self.node(cur, j + 1).position - self.node(cur, j).position).length()
    }

    /// `FUN_004e3ee0`: the segment (from `cur`) that arc length `s` falls in and the fraction along it; beyond the
    /// ring, (count − 1, 0).
    pub fn locate(&self, cur: usize, s: f32) -> (usize, f32) {
        if s <= 0.0 {
            return (0, 0.0);
        }
        let segments = self.count_from(cur).saturating_sub(1);
        let mut acc = 0.0;
        for j in 0..segments {
            let len = self.segment_length(cur, j);
            if s < acc + len {
                return (j, (s - acc) / len);
            }
            acc += len;
        }
        (segments, 0.0)
    }

    /// `FUN_004e8190`: the Hermite point at arc length `s` from node `cur` (fraction = the chord fraction of
    /// `locate`); beyond the ring, the last node.
    pub fn point_at(&self, cur: usize, s: f32) -> V2 {
        let (k, u) = self.locate(cur, s);
        if k + 1 < self.count_from(cur) {
            let (a, b) = (self.node(cur, k), self.node(cur, k + 1));
            let (u2, u3) = (u * u, u * u * u);
            let h00 = u3 * 2.0 - u2 * 3.0 + 1.0;
            let h10 = u3 - u2 * 2.0 + u;
            let h01 = u2 * 3.0 - u3 * 2.0;
            let h11 = u3 - u2;
            a.position * h00 + a.tangent * h10 + b.position * h01 + b.tangent * h11
        } else {
            self.node(cur, k).position
        }
    }

    /// The car's distance along the current segment (`FUN_00505e60`, `FUN_0050a740`): (pos − node) · the unit
    /// segment direction.
    pub fn along(&self, cur: usize, pos: V2) -> f32 {
        let a = self.node(cur, 0).position;
        let d = (self.node(cur, 1).position - a).normalize();
        (pos - a).dot(d)
    }

    /// The node step of `FUN_0050a740` → `FUN_00512b70`: the car past the end of its segment moves `cur` to the
    /// next node (one step per tick).
    pub fn advance(&self, cur: usize, pos: V2) -> usize {
        if self.count_from(cur) < 2 {
            return cur;
        }
        if self.segment_length(cur, 0) < self.along(cur, pos) {
            if self.closed { (cur + 1) % self.nodes.len() } else { cur + 1 }
        } else {
            cur
        }
    }

    /// `FUN_004e47f0`: the car's arc length along the ring from `cur`: walk the segments from the ring start until
    /// the car's projection on one falls short of its length; the arc length is the segments passed plus that
    /// projection (the decompiler lost the x87 return; the walk is read). A car behind the first node within 1 m
    /// gets 0; past the last segment, the ring's length.
    pub fn project(&self, cur: usize, pos: V2) -> f32 {
        let segments = self.count_from(cur).saturating_sub(1);
        let mut acc = 0.0;
        for j in 0..segments {
            let a = self.node(cur, j).position;
            let ab = self.node(cur, j + 1).position - a;
            let len = ab.length();
            let d = if len > 0.0 { ab * (1.0 / len) } else { ab };
            let t = (pos - a).dot(d);
            if j == 0 && t < 0.0 && (pos - a).dot(pos - a) < 1.0 {
                return 0.0;
            }
            if t < len {
                return acc + t;
            }
            acc += len;
        }
        acc
    }

    /// The unit chord direction of the segment arc length `s` falls in (ours for `FUN_004e3c30`'s tangent output,
    /// which only orients the lane offset).
    pub fn direction_at(&self, cur: usize, s: f32) -> V2 {
        let (k, _) = self.locate(cur, s);
        let k = k.min(self.count_from(cur).saturating_sub(2));
        (self.node(cur, k + 1).position - self.node(cur, k).position).normalize()
    }

    /// Ours, for placing a driver: the segment start nearest to `pos` (the game's path builder starts the ring
    /// at the car).
    pub fn nearest_segment(&self, pos: V2) -> usize {
        let n = self.nodes.len();
        let segs = if self.closed { n } else { n.saturating_sub(1) };
        let mut best = (f32::MAX, 0);
        for k in 0..segs {
            let a = self.nodes[k].position;
            let b = self.nodes[(k + 1) % n].position;
            let ab = b - a;
            let t = ((pos - a).dot(ab) / ab.dot(ab).max(1e-6)).clamp(0.0, 1.0);
            let d = (pos - (a + ab * t)).length();
            if d < best.0 {
                best = (d, k);
            }
        }
        best.1
    }

    /// `FUN_004e4eb0`: the larger of the left and right turn sums over the path ahead of the car, up to
    /// `distance` metres. The first term is wrap(angle(heading) − angle(current segment)) (the reverse sense of
    /// the node-to-node terms; kept as the game has it); each next term wrap(angle(segment k) − angle(segment
    /// k − 1)), negative ones summed into one side, positive into the other. The distance starts at the current
    /// segment's length plus the car's distance to its end and grows by each next segment; the loop stops once it
    /// reaches `distance`. Radians.
    pub fn turn_ahead(&self, cur: usize, pos: V2, heading: V2, distance: f32) -> f32 {
        let count = self.count_from(cur);
        if count < 2 {
            return 0.0;
        }
        let (p0, p1) = (self.node(cur, 0).position, self.node(cur, 1).position);
        let back = (p0 - p1).normalize();
        let mut neg = 0.0f32;
        let mut pos_sum = 0.0f32;
        let first = wrap_angle(heading.angle() - (p1 - p0).normalize().angle());
        if first < 0.0 {
            neg = -first;
        } else {
            pos_sum = first;
        }
        let mut covered = (p1 - p0).length() + (pos - p1).dot(back);
        let mut k = 1;
        while k < count - 1 {
            if distance <= covered {
                break;
            }
            let prev = (self.node(cur, k).position - self.node(cur, k - 1).position).normalize();
            let next = self.node(cur, k + 1).position - self.node(cur, k).position;
            let d = wrap_angle(next.normalize().angle() - prev.angle());
            if d < 0.0 {
                neg -= d;
            } else {
                pos_sum += d;
            }
            covered += next.length();
            k += 1;
        }
        if pos_sum < neg { neg } else { pos_sum }
    }

    /// `FUN_00503880`: the look-ahead time. Over the nodes within `look_scan_time` × speed metres of the current
    /// node (`locate`, one more when the scan ends inside a segment) the turns are summed; the scan stops at the
    /// first node whose inside clearance (+0x38 when the sum is positive, else +0x3c) is under `corner_clearance`
    /// and that turns more than `sharp_turn` (or has flag 0x80); d = the path length from the current node to it
    /// (to the last node scanned without one). T = d / speed, at least `min_look_time`, at most the lane planner's
    /// cap (driver +0x549c, `cap`; 1000 = none, `.rdata` 0x00fa8bd4) and at most `look_scan_time`.
    pub fn look_time(&self, cfg: &AiConfig, cur: usize, speed: f32, cap: Option<f32>) -> f32 {
        let speed = speed.abs();
        let (mut last, frac) = self.locate(cur, cfg.look_scan_time * speed);
        if frac > 0.0 {
            last += 1;
        }
        let sum: f32 = (0..=last).map(|j| self.node(cur, j).turn).sum();
        let side = if sum <= 0.0 { 1 } else { 0 };
        let sharp = cfg.sharp_turn * 0.017_453_3; // `.rdata` 0x00fa88cc
        let stop = (0..=last)
            .find(|&j| {
                let n = self.node(cur, j);
                n.clearance[side] < cfg.corner_clearance && (n.turn.abs() > sharp || n.flags & 0x80 != 0)
            })
            .unwrap_or(last);
        let d: f32 = (0..stop).map(|j| self.segment_length(cur, j)).sum();
        let mut t = (d / speed).max(cfg.min_look_time);
        if let Some(c) = cap
            && c != 1000.0
            && c < t
        {
            t = c;
        }
        if t <= cfg.look_scan_time { t } else { cfg.look_scan_time }
    }
}

/// The path's corner speeds (m/s per node), the game's corner annotation (`FUN_004ea510` finds the corners,
/// `FUN_004ea860` sets the node speed byte +0x4d), read only by the virtual progress of a racer without a car
/// (`FUN_005056b0`). A corner is a run of same-sign turns passing 3° (`.rdata` 0x00fa884c) and totalling 25°
/// (0x00fa8848); its ends are where 3° is reached from either side; each half (≥ 5 m) gets one speed: with the
/// chord L of the half (the road's full width instead when that is over 1.5 L, 0x00fa8850) and Δ the heading change
/// since the corner's entry segment, R = (L / 2) / cos((π − Δ) / 2) for every segment with Δ > 0.01, and the
/// half's speed = clamp(`scale` × min(top speed, √(9.81 μ R)), 0, 255). Nodes outside corners stay 255.
pub fn corner_speeds(points: &[V2], turn: &[f32], width: f32, top_speed: f32, mu: f32, scale: f32) -> Vec<f32> {
    let n = points.len();
    let mut speed = vec![255.0; n];
    if n < 3 {
        return speed;
    }
    let heading = |k: usize| {
        let d = points[(k + 1).min(n - 1)] - points[k.min(n - 2)];
        d.x.atan2(d.z)
    };
    let (start, total) = (3f32.to_radians(), 25f32.to_radians());
    let mut k = 1;
    while k < n - 1 {
        let sign = turn[k].signum();
        if turn[k] == 0.0 {
            k += 1;
            continue;
        }
        let mut end = k;
        while end + 1 < n - 1 && turn[end + 1] != 0.0 && turn[end + 1].signum() == sign {
            end += 1;
        }
        if turn[k..=end].iter().sum::<f32>().abs() >= total {
            let mut acc = 0.0;
            let mut s = k;
            for (j, t) in turn.iter().enumerate().take(end + 1).skip(k) {
                acc += t.abs();
                if acc > start {
                    s = j;
                    break;
                }
            }
            acc = 0.0;
            let mut e = end;
            for j in (k..=end).rev() {
                acc += turn[j].abs();
                if acc > start {
                    e = j;
                    break;
                }
            }
            if e > s {
                let (mid, mid2) = ((s + e) / 2, (s + e).div_ceil(2));
                let theta0 = heading(s.saturating_sub(1));
                for (a, b) in [(s, mid), (mid2.max(mid + 1).min(e), e)] {
                    let chord = (points[b] - points[a]).length();
                    if chord * chord < 25.0 || b <= a {
                        continue;
                    }
                    let l = if width > 1.5 * chord { width } else { chord };
                    let mut v_min = top_speed;
                    for seg in a..b {
                        let delta = wrap_angle(theta0 - heading(seg)).abs();
                        if delta > 0.01 {
                            let r = (l * 0.5) / ((std::f32::consts::PI - delta) * 0.5).cos();
                            v_min = v_min.min((9.81 * mu * r).sqrt());
                        }
                    }
                    let byte = (scale * v_min).clamp(0.0, 255.0);
                    for sp in &mut speed[a..=b] {
                        *sp = byte;
                    }
                }
            }
        }
        k = end + 1;
    }
    speed
}

/// `FUN_00518520`: the braking distance from `v0` to `v1` (m/s) for a deceleration a·v² + b:
/// 1.666 ln((a v0² + b) / (a v1² + b)) / (2a); for a = 0 the limit 1.666 (v0² − v1²) / (2b). The game's
/// a = (vehicle+0x688 × −vehicle+0x7d8 + vehicle+0x7d4) / mass, b = vehicle+0x688 × 9.81 (+0x688's writer not
/// found).
pub fn braking_distance(v0: f32, v1: f32, a: f32, b: f32) -> f32 {
    if a.abs() < 1e-6 {
        1.666 * (v0 * v0 - v1 * v1) / (2.0 * b)
    } else {
        1.666 * ((a * v0 * v0 + b) / (a * v1 * v1 + b)).ln() / (2.0 * a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(n: usize) -> Path {
        Path::new(&(0..n).map(|k| V2::new(0.0, 15.0 * k as f32)).collect::<Vec<_>>(), false)
    }

    #[test]
    fn hermite_passes_through_the_nodes_and_clamps_at_the_end() {
        let p = straight(5);
        assert!((p.point_at(0, 15.0) - V2::new(0.0, 15.0)).length() < 1e-4);
        assert!((p.point_at(0, 22.5) - V2::new(0.0, 22.5)).length() < 1e-3);
        assert_eq!(p.point_at(0, 1000.0), V2::new(0.0, 60.0));
        assert_eq!(p.locate(0, 1000.0), (4, 0.0));
    }

    #[test]
    fn turn_sign_right_is_positive() {
        // North (+Z), then east... +X is the left of a car facing +Z, so a turn towards −X is a right turn.
        let p = Path::new(&[V2::new(0.0, 0.0), V2::new(0.0, 15.0), V2::new(-15.0, 15.0)], false);
        assert!(p.nodes[1].turn > 1.5, "{}", p.nodes[1].turn);
        let q = Path::new(&[V2::new(0.0, 0.0), V2::new(0.0, 15.0), V2::new(15.0, 15.0)], false);
        assert!(q.nodes[1].turn < -1.5);
    }

    #[test]
    fn look_time_shortens_at_a_tight_corner() {
        let cfg = AiConfig::default();
        let mut pts: Vec<V2> = (0..4).map(|k| V2::new(0.0, 15.0 * k as f32)).collect();
        pts.extend((1..4).map(|k| V2::new(-15.0 * k as f32, 45.0)));
        let mut p = Path::new(&pts, false);
        // Clear road: the scan's full time.
        assert_eq!(p.look_time(&cfg, 0, 100.0, None), cfg.look_scan_time);
        // A narrow 90° corner at node 3 (45 m): T = 45 / speed.
        p.nodes[3].clearance = [4.0, 4.0];
        p.nodes[3].flags = 0x80;
        let t = p.look_time(&cfg, 0, 100.0, None);
        assert!((t - 0.45).abs() < 1e-6, "{t}");
        assert!((p.look_time(&cfg, 0, 90.0, None) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn turn_ahead_sums_both_sides() {
        let pts = [V2::new(0.0, 0.0), V2::new(0.0, 15.0), V2::new(-15.0, 30.0), V2::new(0.0, 45.0)];
        let p = Path::new(&pts, false);
        let t = p.turn_ahead(0, V2::new(0.0, 0.0), V2::new(0.0, 1.0), 100.0);
        // 45° right then 90° left: the larger side.
        assert!((t - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{t}");
        // Short distance: only the heading term (0) and the first bend.
        let s = p.turn_ahead(0, V2::new(0.0, 0.0), V2::new(0.0, 1.0), 10.0);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn advance_steps_past_the_segment_end() {
        let p = straight(4);
        assert_eq!(p.advance(0, V2::new(0.0, 14.0)), 0);
        assert_eq!(p.advance(0, V2::new(0.0, 16.0)), 1);
    }
}
