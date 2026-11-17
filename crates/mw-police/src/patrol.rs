//! A patrolling cop (driver state 0xd Patrolling, update `FUN_005101e0`; docs/decomp.md "How a cop patrols
//! (2026-10-16)"). The game drives a patrolling cop with traffic: the cop asks the traffic system for a **ghost**,
//! an ordinary lane follower that is never drawn and has no body (car +0x78 = 2), and once it has caught up the
//! cop's car is put on the ghost's pose every frame. So a patrolling cop keeps to its lane, takes junctions the
//! way traffic does (least loaded span), stops at red lights and behind the car ahead, and never overtakes.
//!
//! Modes (byte +0x90):
//! 0. *Find* (`FUN_00501460`): the nearest lane point to the cop (or, after a search, to a point 150 m away in a
//!    random direction), any lane direction; within 400 m and 5 m of height first, then without the height test,
//!    then within 1000 m, one widening per failed frame (+0x78).
//! 1. *To the lane* (`FUN_0050bdd0`): drive there over the road graph; every 4 s the point is looked up again and
//!    taken when it moved more than 30 m; within 30 m the speed ramps down to 0 from 10 mph, within 6 m the cop
//!    stops on the handbrake; within 8 m: mode 2.
//! 2. *Ask for a ghost* (`FUN_00509df0`): stopped; traffic message 0x1a places the ghost at the nearest rung
//!    within 20 m of the point 5 m ahead of the cop (`FUN_0060aba0`); asked again after 2 s without one.
//! 3. *Merge* (`FUN_0050ce00`): drive to the ghost; while the cop is more than 15 m behind it the ghost crawls at
//!    10 mph (override 3), when it is behind the cop it drives normally; caught up (the cop 1.5 s ahead within
//!    2 m of it, the ghost ahead): mode 4. More than 40 m from the player and off screen: mode 4 at once. After
//!    8 s without catching up: mode 0 again.
//! 4. *Ride* (`FUN_0050d490`): within 5 m of the ghost the cop's body is set to the ghost's pose and speed
//!    (physics message 0x2e); the ghost gone: mode 0.

use glam::{Vec2, Vec3};

/// The traffic system's side of the patrol (the runtime's `traffic.rs`; the game's traffic messages 0x1a / 0x1b /
/// 0x1c and its lane lookup `FUN_005f3350`).
pub trait PatrolTraffic {
    /// A ghost's handle.
    type Ghost: Copy + PartialEq + core::fmt::Debug;
    /// The nearest lane point within `radius` (and `max_dy` of height), any lane direction; `active_only` = in an
    /// active hull.
    fn nearest_lane(&self, point: Vec3, radius: f32, max_dy: Option<f32>, active_only: bool) -> Option<Vec3>;
    /// Message 0x1a: a ghost at the nearest rung within 20 m of `at` (the traffic tick `FUN_0060aba0`).
    fn add_ghost(&mut self, at: Vec3, cop: u32, half_length: f32) -> Option<Self::Ghost>;
    /// The ghost's pose, while it exists.
    fn ghost(&mut self, ghost: Self::Ghost) -> Option<GhostPose>;
    /// Message 0x1b: the ghost's speed override (`FUN_0061a450`: 0 none, 1 stop, 2 5 mph, 3 10 mph).
    fn set_ghost_override(&mut self, ghost: Self::Ghost, value: u8);
    /// Message 0x1c: release the ghost.
    fn remove_ghost(&mut self, ghost: Self::Ghost);
    /// The ghost's lane path ahead, `ahead` metres (for the cop's steering).
    fn ghost_path(&self, ghost: Self::Ghost, ahead: f32) -> Vec<Vec3>;
}

/// A ghost's pose for its cop.
#[derive(Debug, Clone, Copy)]
pub struct GhostPose {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub speed: f32,
    /// Its lane's cruise speed (m/s; `TrafficFlow.Speed`).
    pub cruise: f32,
}

const MPH: f32 = 0.44704;

/// Mode 0's lane search radii and height test by the failure count (`FUN_00501460` → `FUN_004fd790`).
const FIND: [(f32, Option<f32>); 4] = [(400.0, Some(5.0)), (400.0, None), (1000.0, Some(5.0)), (1000.0, None)];
/// `.rdata` 0x00fa8eec: the wander point's distance (m) after a search; points within 60 m of the player's car
/// are not taken.
const WANDER: f32 = 150.0;
const WANDER_CLEAR: f32 = 60.0;
/// Mode 1: the lane point is looked up again every 4 s (0x00fa92b8) and taken when 30 m off (900 at
/// 0x010c4344); within 8 m (64 at 0x010c4338) mode 2; within 30 m the speed ramps down from 10 mph; within 6 m
/// the handbrake.
const REQUERY: f32 = 4.0;
const RETARGET: f32 = 30.0;
const ARRIVED: f32 = 8.0;
const RAMP: f32 = 30.0;
const RAMP_MPH: f32 = 10.0;
const HANDBRAKE: f32 = 6.0;
/// Mode 2: the ghost is asked for 5 m ahead of the cop (0x00fa92f8), again after 2 s (0x00f8a808).
const ASK_AHEAD: f32 = 5.0;
const ASK_AGAIN: f32 = 2.0;
/// Mode 3: 8 s to catch up (0x00fa8e64); the ghost crawls while the cop is more than 15 m from it (225 at
/// 0x010c4340); caught up when the cop 1.5 s ahead is within 2 m of it (4 at 0x010c433c); the unseen shortcut
/// beyond 40 m of the player (1600 at 0x010c4348).
const MERGE_TIME: f32 = 8.0;
const CRAWL_BEYOND: f32 = 15.0;
const CAUGHT_LEAD: f32 = 1.5;
const CAUGHT: f32 = 2.0;
const UNSEEN: f32 = 40.0;
/// MODEL: the cop's speed while merging is the lane's cruise speed, at least `MERGE_MIN_MPH` (the game's target is
/// `.bss` 0x011061f8, not read: needs a live read; the 8 s limit with a ghost that starts at the lane's speed and
/// then crawls at 10 mph asks for about that).
const MERGE_MIN_MPH: f32 = 20.0;
/// Mode 4: the body is put on the ghost while within 5 m of it (25 at 0x010c434c).
const RIDE_RANGE: f32 = 5.0;

/// The patrol's mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatrolMode<G> {
    Find { fails: usize, wander: bool },
    ToLane { target: Vec3, requery: f32 },
    Ask { timer: f32 },
    Merge { ghost: G, timer: f32 },
    Ride { ghost: G },
}

impl<G> Default for PatrolMode<G> {
    fn default() -> Self {
        Self::Find { fails: 0, wander: false }
    }
}

impl<G: Copy> PatrolMode<G> {
    /// The mode number (+0x90).
    pub fn number(&self) -> u8 {
        match self {
            Self::Find { .. } => 0,
            Self::ToLane { .. } => 1,
            Self::Ask { .. } => 2,
            Self::Merge { .. } => 3,
            Self::Ride { .. } => 4,
        }
    }
    pub fn ghost(&self) -> Option<G> {
        match *self {
            Self::Merge { ghost, .. } | Self::Ride { ghost } => Some(ghost),
            _ => None,
        }
    }
}

/// One tick's view of the cop.
pub struct PatrolInput {
    pub dt: f32,
    /// The cop's id (the ghost's owner).
    pub cop: u32,
    pub position: Vec3,
    pub forward: Vec3,
    pub velocity: Vec3,
    pub player: Vec3,
    /// The cop's 4 m sphere is in the camera's view (`FUN_006bf380`).
    pub on_screen: bool,
    /// Half the cop car's length (the ghost's box, MODEL).
    pub half_length: f32,
    /// A uniform random number in [0, 1) (the wander direction).
    pub random: f32,
}

/// What the cop's driver does this tick.
#[derive(Debug, Clone)]
pub enum PatrolDrive {
    /// Over the road graph to `target` at up to `speed` (m/s), on the handbrake when asked.
    Road { target: Vec3, speed: f32, handbrake: bool },
    /// Stand.
    Stop,
    /// Along `path` at `speed` (to the ghost).
    Path { path: Vec<Vec3>, speed: f32 },
    /// On the ghost: the body follows `pose`; `snap` = put it there at once; `path` for the steering.
    Ride { pose: GhostPose, path: Vec<Vec3>, snap: bool },
}

impl<G: Copy> PatrolMode<G> {
    /// Release the ghost (the state's exit `FUN_00506120`, message 0x1c).
    pub fn leave<T: PatrolTraffic<Ghost = G>>(&mut self, traffic: &mut T) {
        if let Some(g) = self.ghost() {
            traffic.remove_ghost(g);
        }
        *self = Self::default();
    }

    /// One tick (`FUN_005101e0`).
    pub fn tick<T: PatrolTraffic<Ghost = G>>(&mut self, i: &PatrolInput, traffic: &mut T) -> PatrolDrive {
        let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length();
        match *self {
            Self::Find { fails, wander } => {
                let query = if wander {
                    let theta = i.random * std::f32::consts::TAU;
                    i.position + Vec3::new(theta.sin(), 0.0, theta.cos()) * WANDER
                } else {
                    i.position
                };
                let (radius, dy) = FIND[fails.min(FIND.len() - 1)];
                match traffic.nearest_lane(query, radius, dy, false) {
                    Some(l) if !(wander && l.distance(i.player) < WANDER_CLEAR) => {
                        *self = Self::ToLane { target: l, requery: REQUERY };
                    }
                    _ => *self = Self::Find { fails: fails + 1, wander },
                }
                PatrolDrive::Stop
            }
            Self::ToLane { mut target, mut requery } => {
                requery -= i.dt;
                if requery <= 0.0 {
                    requery = REQUERY;
                    if let Some(l) = traffic.nearest_lane(i.position, FIND[0].0, FIND[0].1, false)
                        && l.distance(target) > RETARGET
                    {
                        target = l;
                    }
                }
                let d = flat(i.position, target);
                if d < ARRIVED {
                    *self = Self::Ask { timer: 0.0 };
                    return PatrolDrive::Stop;
                }
                *self = Self::ToLane { target, requery };
                // MODEL: beyond the ramp the speed limit is the level's go-to / search cap (the game's
                // `FUN_00508d50` reads D+0x5668, not traced for this mode); `police.rs` applies it.
                let speed = if d < RAMP { RAMP_MPH * MPH * d / RAMP } else { f32::MAX };
                PatrolDrive::Road { target, speed, handbrake: d < HANDBRAKE }
            }
            Self::Ask { timer } => {
                if timer <= 0.0 {
                    let at = i.position + i.forward * ASK_AHEAD;
                    if let Some(ghost) = traffic.add_ghost(at, i.cop, i.half_length) {
                        *self = Self::Merge { ghost, timer: 0.0 };
                        return self.tick(i, traffic);
                    }
                    *self = Self::Ask { timer: ASK_AGAIN };
                } else {
                    *self = Self::Ask { timer: timer - i.dt };
                }
                PatrolDrive::Stop
            }
            Self::Merge { ghost, timer } => {
                let Some(pose) = traffic.ghost(ghost) else {
                    *self = Self::default();
                    return PatrolDrive::Stop;
                };
                let timer = timer + i.dt;
                if timer > MERGE_TIME {
                    traffic.remove_ghost(ghost);
                    *self = Self::default();
                    return PatrolDrive::Stop;
                }
                // Unseen and away from the player: onto the ghost at once.
                if flat(i.position, i.player) > UNSEEN && !i.on_screen {
                    traffic.set_ghost_override(ghost, 0);
                    *self = Self::Ride { ghost };
                    return self.ride(i, traffic, ghost, pose);
                }
                let to = pose.position - i.position;
                let ahead = to.dot(i.forward) > 0.0;
                if !ahead {
                    traffic.set_ghost_override(ghost, 0);
                } else if to.length() > CRAWL_BEYOND {
                    traffic.set_ghost_override(ghost, 3);
                }
                let lead = i.position + i.velocity * CAUGHT_LEAD;
                // MODEL: a cop within the ride range of the ghost has caught up too (the game's 1.5 s lead test alone
                // needs the cop to come from farther back than its own lead).
                let alongside = to.length() < RIDE_RANGE;
                if (ahead && lead.distance(pose.position) < CAUGHT) || alongside {
                    traffic.set_ghost_override(ghost, 0);
                    *self = Self::Ride { ghost };
                    return self.ride(i, traffic, ghost, pose);
                }
                *self = Self::Merge { ghost, timer };
                let mut path = vec![i.position];
                path.extend(traffic.ghost_path(ghost, 60.0));
                PatrolDrive::Path { path, speed: pose.cruise.max(MERGE_MIN_MPH * MPH) }
            }
            Self::Ride { ghost } => {
                let Some(pose) = traffic.ghost(ghost) else {
                    *self = Self::default();
                    return PatrolDrive::Stop;
                };
                self.ride(i, traffic, ghost, pose)
            }
        }
    }

    fn ride<T: PatrolTraffic<Ghost = G>>(
        &mut self,
        i: &PatrolInput,
        traffic: &mut T,
        ghost: G,
        pose: GhostPose,
    ) -> PatrolDrive {
        let path = traffic.ghost_path(ghost, 60.0);
        if pose.position.distance(i.position) <= RIDE_RANGE {
            return PatrolDrive::Ride { pose, path, snap: false };
        }
        // MODEL: farther off, the game posts a pose request to the driver (D+0x584c, its reader not traced):
        // unseen and away from the player the cop is put on the ghost; otherwise it starts over.
        let away = Vec2::new(i.position.x - i.player.x, i.position.z - i.player.z).length() > UNSEEN;
        if away && !i.on_screen {
            return PatrolDrive::Ride { pose, path, snap: true };
        }
        traffic.remove_ghost(ghost);
        *self = Self::default();
        PatrolDrive::Stop
    }
}
