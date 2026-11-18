//! `--test cops`: when a pursuit starts (`cop_sight.rs`; docs/decomp.md "When a pursuit starts"). One patrolling
//! cop stands at the side of a straight road, facing the player's car down it; the player's car (driven by the AI
//! racers' driver) then:
//!
//! 1. *slow pass*: drives past the cop at 30 mph: the cop sees it, no pursuit;
//! 2. *speeding*: drives past at 125 mph (over level 0's 110 mph limit): wanted for speeding, chase;
//! 3. *ram*: drives into the cop at 35 mph: the cop is rammed, chase.
//!
//! Between the cases the pursuit is reset and the cop put back. Two traffic hits in a cop's view are covered by
//! `cop_sight`'s unit tests (traffic cannot be placed reliably here).
//!
//! 4. *patrol*: the cop is let go on a traffic lane ahead of the player's car, which follows it slowly: the cop
//!    gets its ghost and rides it in the lane (`cop_patrol.rs`); no pursuit. `MW_COPS_SHOTS=<dir>` saves
//!    screenshots of it.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{Cop, CopMode, MPH, Police};
use crate::ai::{AiDriver, AiRoute};
use crate::cop_patrol::Lanes;
use crate::cop_sight::{Perception, Wanted};
use crate::drive::{Car, ExternalControls, Player};
use crate::pursuit::PursuitState;
use mw_police::coordinator::Role;

/// The straight stretch the cases run on (m), the cop's place along it and its offset to the side.
const STRAIGHT: f32 = 220.0;
const COP_ALONG: f32 = 140.0;
const COP_SIDE: f32 = 6.0;
/// A case ends this long after the player's car has passed the cop, or after this long in all.
const AFTER_PASS: f32 = 6.0;
const CASE_LIMIT: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Case {
    SlowPass,
    Speeding,
    Ram,
    Patrol,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Case::SlowPass => "slow-pass",
            Case::Speeding => "speeding",
            Case::Ram => "ram",
            Case::Patrol => "patrol",
        }
    }
    fn mph(self) -> f32 {
        match self {
            Case::SlowPass => 30.0,
            Case::Speeding => 125.0,
            Case::Ram => 35.0,
            Case::Patrol => 0.0,
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct StartTest {
    phase: u8,
    timer: f32,
    case: usize,
    /// The road: start of the straight stretch and its direction.
    road: Option<(Arc<AiRoute>, f32)>,
    /// What happened in the current case.
    saw: bool,
    closest: f32,
    wanted: Option<crate::cop_sight::WantedReason>,
    wanted_at: Option<(f32, f32, f32)>,
    chase_at: Option<f32>,
    passed_at: Option<f32>,
    results: Vec<(Case, bool)>,
    /// The patrol phase: seconds riding, the largest distance from the lane while riding, shots taken.
    riding: f32,
    lane_off: f32,
    shots: u32,
    patrol_modes: Vec<u8>,
}

const CASES: [Case; 3] = [Case::SlowPass, Case::Speeding, Case::Ram];

/// A straight stretch of `STRAIGHT` metres on a road-graph route from the player's car: the start distance.
fn find_straight(route: &AiRoute) -> Option<f32> {
    let mut s0 = 30.0;
    while s0 + STRAIGHT < route.length() {
        let (_, d0) = route.at(s0);
        let straight = (0..=(STRAIGHT / 10.0) as usize).all(|k| {
            let (_, d) = route.at(s0 + k as f32 * 10.0);
            d.dot(d0) > 0.998 && (d.y - d0.y).abs() < 0.05
        });
        if straight {
            return Some(s0);
        }
        s0 += 10.0;
    }
    None
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn run_start_test(
    mut commands: Commands,
    time: Res<Time>,
    mut test: ResMut<StartTest>,
    mut police: ResMut<Police>,
    mut player: Query<
        (Entity, &mut Transform, &mut LinearVelocity, &mut AngularVelocity, &mut crate::drive::CarControls),
        (With<Player>, With<Car>, Without<Cop>),
    >,
    mut cops: Query<
        (Entity, &mut Transform, &mut LinearVelocity, &mut AngularVelocity, &mut Cop, &mut AiDriver),
        Without<Player>,
    >,
    mut traffic: Option<ResMut<crate::traffic::TrafficSystem>>,
    mut exit: MessageWriter<AppExit>,
) {
    let police = &mut *police;
    let Ok((me, mut pt, mut pv, mut pw, mut controls)) = player.single_mut() else { return };
    let dt = time.delta_secs();
    test.timer += dt;
    let Some(graph) = police.graph.clone() else {
        if test.timer > 30.0 {
            info!("TEST RESULT cops: no road graph");
            exit.write(AppExit::Success);
        }
        return;
    };
    match test.phase {
        // Find the road, ask for one cop close by and hold it.
        0 => {
            police.test_hold = true;
            police.spawn_distance_override = Some(60.0);
            police.spawn_block = 0.0;
            let fwd = pt.rotation * Vec3::Z;
            let Some(route) = crate::ai::route_from_ai_graph(&graph, pt.translation, fwd.x.atan2(fwd.z), 3000.0) else {
                info!("TEST RESULT cops: no road at the player's car");
                exit.write(AppExit::Success);
                return;
            };
            let Some(s0) = find_straight(&route) else {
                info!("TEST RESULT cops: no straight road within 3 km");
                exit.write(AppExit::Success);
                return;
            };
            info!("cops test: straight road from {:.0?} ({s0:.0} m along)", route.at(s0).0);
            test.road = Some((Arc::new(route), s0));
            commands.entity(me).insert(ExternalControls);
            test.phase = 1;
            test.timer = 0.0;
        }
        // Wait for the cop.
        1 => {
            pv.0 = Vec3::ZERO;
            if cops.iter().next().is_some() && test.timer > 2.0 {
                police.hold_spawns = true;
                test.phase = 2;
                test.timer = 0.0;
            } else if test.timer > 60.0 {
                info!("TEST RESULT cops: TIMEOUT waiting for a cop");
                exit.write(AppExit::Success);
            }
        }
        // Set up a case.
        2 => {
            let Some(case) = CASES.get(test.case).copied() else {
                if test.phase == 2 {
                    test.phase = 4;
                    test.timer = 0.0;
                    return;
                }
                let passed = test.results.iter().filter(|r| r.1).count();
                info!(
                    "TEST RESULT cops: {passed}/{} cases as the game's rules say ({})",
                    test.results.len(),
                    test.results
                        .iter()
                        .map(|(c, ok)| format!("{} {}", c.name(), if *ok { "ok" } else { "FAIL" }))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                exit.write(AppExit::Success);
                return;
            };
            let (route, s0) = test.road.clone().expect("found in phase 0");
            let (start, dir) = route.at(s0);
            let (cop_at, _) = route.at(s0 + COP_ALONG);
            let right = dir.cross(Vec3::Y).normalize_or_zero();
            let cop_pos = cop_at + right * COP_SIDE + Vec3::Y * 0.3;
            // No traffic on the stretch (a hit would slow the car under the case's speed).
            if let Some(tr) = traffic.as_mut() {
                tr.clear_near(route.at(s0 + COP_ALONG * 0.5).0, STRAIGHT, &mut commands);
            }
            // The pursuit and what the cops know start afresh.
            police.core.pursuit.reset();
            police.wanted = Wanted::default();
            police.was_pursuit = false;
            for (e, mut t, mut v, mut w, mut cop, mut driver) in &mut cops {
                *t = Transform::from_translation(cop_pos).with_rotation(Quat::from_rotation_y((-dir.x).atan2(-dir.z)));
                commands.entity(e).insert((Position(t.translation), Rotation(t.rotation)));
                v.0 = Vec3::ZERO;
                w.0 = Vec3::ZERO;
                cop.mode = CopMode::Patrol;
                cop.touching = false;
                if let Some(b) = police.core.cop_mut(cop.slot as u32) {
                    b.role = Role::Patrol;
                    b.sight = Perception::default();
                    b.driver.reassign();
                }
                // A fresh driver on a short line where it stands (the old route's recovery would put it back).
                let mut fresh = AiDriver::new(Arc::new(AiRoute::new(vec![cop_pos, cop_pos - dir * 50.0], false)));
                fresh.ignore = driver.ignore;
                fresh.keep_wrecked = true;
                *driver = fresh;
            }
            let speed = case.mph() * MPH;
            *pt = Transform::from_translation(start + Vec3::Y * 0.3)
                .with_rotation(Quat::from_rotation_y(dir.x.atan2(dir.z)));
            commands.entity(me).insert((Position(pt.translation), Rotation(pt.rotation)));
            pv.0 = dir * speed;
            pw.0 = Vec3::ZERO;
            // The car's route: on along the road, or straight at the cop for the ram.
            let path = if case == Case::Ram {
                let to = (cop_pos - start).normalize_or(dir);
                AiRoute::new(vec![start, cop_pos, cop_pos + to * 200.0], false)
            } else {
                AiRoute::new((0..=60).map(|k| route.at(s0 + k as f32 * 10.0).0).collect(), false)
            };
            let mut driver = AiDriver::new(Arc::new(path));
            driver.schedule_speed = speed;
            commands.entity(me).insert(driver);
            info!("cops test: case {} at {:.0} mph, the cop at {cop_pos:.0?}", case.name(), case.mph());
            test.saw = false;
            test.closest = f32::MAX;
            test.wanted = None;
            test.wanted_at = None;
            test.chase_at = None;
            test.passed_at = None;
            test.phase = 3;
            test.timer = 0.0;
        }
        // Run it.
        3 => {
            let case = CASES[test.case];
            let (route, s0) = test.road.clone().expect("found in phase 0");
            let mph = pv.0.length() / MPH;
            for (_, t, _, _, cop, _) in &cops {
                test.saw |= police.core.cop(cop.slot as u32).is_some_and(|b| b.sight.sees);
                let d = t.translation.distance(pt.translation);
                test.closest = test.closest.min(d);
            }
            if test.wanted.is_none()
                && let Some(r) = police.wanted.reason
            {
                let d = cops.iter().map(|c| c.1.translation.distance(pt.translation)).fold(f32::MAX, f32::min);
                test.wanted = Some(r);
                test.wanted_at = Some((test.timer, mph, d));
            }
            if test.chase_at.is_none() && police.pursuit.state == PursuitState::Chase {
                test.chase_at = Some(test.timer);
            }
            let (s, _) = route.project(pt.translation, Some(s0 + COP_ALONG), 100.0);
            if test.passed_at.is_none() && (s > s0 + COP_ALONG + 5.0 || (case == Case::Ram && test.closest < 4.0)) {
                test.passed_at = Some(test.timer);
            }
            let done = test.passed_at.is_some_and(|p| test.timer > p + AFTER_PASS)
                || test.timer > CASE_LIMIT
                || (case != Case::SlowPass && test.chase_at.is_some_and(|c| test.timer > c + 1.0));
            if done {
                let chased = test.chase_at.is_some();
                let ok = match case {
                    Case::SlowPass => !chased && test.saw,
                    Case::Speeding => chased && test.wanted == Some(crate::cop_sight::WantedReason::Speeding),
                    Case::Ram => chased && test.wanted == Some(crate::cop_sight::WantedReason::Rammed),
                    Case::Patrol => false,
                };
                info!(
                    "TEST RESULT cops {}: {} (player at {:.0} mph; the cop saw it: {}, closest {:.1} m; wanted: {:?}{}; chase: {}; heat {:.2})",
                    case.name(),
                    if ok { "ok" } else { "FAIL" },
                    case.mph(),
                    test.saw,
                    test.closest,
                    test.wanted,
                    test.wanted_at.map_or(String::new(), |(t, v, d)| format!(
                        " after {t:.1} s at {v:.0} mph, {d:.0} m from the cop"
                    )),
                    test.chase_at.map_or("none".to_string(), |t| format!("after {t:.1} s")),
                    police.pursuit.heat
                );
                test.results.push((case, ok));
                test.case += 1;
                test.phase = 2;
                test.timer = 0.0;
                commands.entity(me).remove::<AiDriver>();
            }
        }
        // The patrol: the cop on a lane ahead, let go; the player's car behind it.
        4 => {
            let Some(tr) = traffic.as_mut() else {
                info!("TEST RESULT cops patrol: no traffic (run without --no-traffic)");
                test.phase = 6;
                return;
            };
            let (route, s0) = test.road.clone().expect("found in phase 0");
            let (p, _) = route.at(s0 + 60.0);
            let Some(lane) = tr.nearest_lane(p, 30.0, Some(5.0), false) else {
                info!("TEST RESULT cops patrol: no lane near {p:.0?}");
                test.phase = 6;
                return;
            };
            police.test_hold = false;
            police.core.pursuit.reset();
            police.wanted = Wanted::default();
            police.was_pursuit = false;
            let dir = lane.direction;
            let cop_pos = lane.position + Vec3::Y * 0.3;
            for (e, mut t, mut v, mut w, mut cop, mut driver) in &mut cops {
                *t = Transform::from_translation(cop_pos).with_rotation(Quat::from_rotation_y(dir.x.atan2(dir.z)));
                commands.entity(e).insert((Position(t.translation), Rotation(t.rotation)));
                v.0 = Vec3::ZERO;
                w.0 = Vec3::ZERO;
                cop.mode = CopMode::Patrol;
                cop.patrol.leave(&mut Lanes { traffic: tr, cop: e });
                if let Some(b) = police.core.cop_mut(cop.slot as u32) {
                    b.role = Role::Patrol;
                    b.sight = Perception::default();
                }
                let mut fresh = AiDriver::new(Arc::new(AiRoute::new(vec![cop_pos, cop_pos + dir * 50.0], false)));
                fresh.ignore = driver.ignore;
                fresh.keep_wrecked = true;
                *driver = fresh;
            }
            let start = lane.position - dir * 30.0 + Vec3::Y * 0.3;
            *pt = Transform::from_translation(start).with_rotation(Quat::from_rotation_y(dir.x.atan2(dir.z)));
            commands.entity(me).insert((Position(pt.translation), Rotation(pt.rotation)));
            pv.0 = Vec3::ZERO;
            pw.0 = Vec3::ZERO;
            // The player's car stands (it only watches).
            commands.entity(me).remove::<AiDriver>();
            info!("cops test: patrol from {cop_pos:.0?} along {dir:.2?}");
            test.riding = 0.0;
            test.lane_off = 0.0;
            test.phase = 5;
            test.timer = 0.0;
        }
        5 => {
            pv.0 = Vec3::ZERO;
            // The player's car stands on the handbrake (no driver writes its controls now).
            controls.throttle = 0.0;
            controls.brake = 0.0;
            controls.steer = 0.0;
            controls.handbrake = true;
            let shots = std::env::var("MW_COPS_SHOTS").ok();
            for (_, t, v, _, cop, _) in &cops {
                let mode = if cop.mode == CopMode::Patrol { cop.patrol.number() } else { 9 };
                if test.patrol_modes.last() != Some(&mode) {
                    test.patrol_modes.push(mode);
                }
                if mode == 4 {
                    test.riding += dt;
                    if let Some(tr) = traffic.as_ref()
                        && let Some(l) = tr.nearest_lane(t.translation, 10.0, Some(5.0), false)
                    {
                        let off = Vec2::new(l.position.x - t.translation.x, l.position.z - t.translation.z).length();
                        test.lane_off = test.lane_off.max(off);
                    }
                }
                if let Some(dir) = &shots
                    && test.timer > 1.5 + test.shots as f32 * 2.0
                    && test.shots < 4
                {
                    let path = format!("{dir}/patrol{}.png", test.shots);
                    info!(
                        "cops test: shot {path}: cop {:.0} m away at {:.0} mph, patrol mode {mode}",
                        t.translation.distance(pt.translation),
                        v.0.length() / MPH
                    );
                    commands
                        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                        .observe(bevy::render::view::screenshot::save_to_disk(path));
                    test.shots += 1;
                }
            }
            if test.timer > 12.0 {
                let ok = test.riding > 3.0 && test.lane_off < 1.5 && police.pursuit.state == PursuitState::None;
                info!(
                    "TEST RESULT cops patrol: {} (patrol modes {:?}, riding the ghost {:.1} s, at most {:.2} m off the lane centre, pursuit {:?})",
                    if ok { "ok" } else { "FAIL" },
                    test.patrol_modes,
                    test.riding,
                    test.lane_off,
                    police.pursuit.state
                );
                test.results.push((Case::Patrol, ok));
                test.phase = 6;
            }
        }
        6 => {
            let passed = test.results.iter().filter(|r| r.1).count();
            info!(
                "TEST RESULT cops: {passed}/{} cases as the game's rules say ({})",
                test.results.len(),
                test.results
                    .iter()
                    .map(|(c, ok)| format!("{} {}", c.name(), if *ok { "ok" } else { "FAIL" }))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
