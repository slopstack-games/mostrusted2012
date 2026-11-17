//! Synthetic pursuits: the free-roam heat table's shape (levels 0–6, thresholds 0 / 2 / 8 / 16 / 26 / 38 / 52, the
//! level-0/1 sight and distances of the shipped data) with one cop.

use mw_assets::police::{Behaviour, CopSlot, HeatLevel, PoliceConfig};
use mw_police::coordinator::{RoadblockQuery, RoadblockTask, Role, Sequencer, TriggerRegion};
use mw_police::glam::Vec3;
use mw_police::pursuit::{PursuitEvent, PursuitMode, PursuitState};
use mw_police::score::ScoreSlot;
use mw_police::{CopState, PlayerState, Police, PoliceInput, RoadState};

const DT: f32 = 1.0 / 60.0;
const COP_TYPE: u32 = 122713;

fn level(n: u8, threshold: f32) -> HeatLevel {
    HeatLevel {
        id: 1000 + n as u32,
        level: n,
        threshold,
        heat_time: if (1..6).contains(&n) { 300.0 + 50.0 * n as f32 } else { -1.0 },
        cooldown_time: if n == 0 { 14.0 } else { 20.0 },
        lose_radius: 150.0,
        cooldown_allowed: true,
        level_up_action: 5000 + n as u32,
        main: Behaviour {
            spawn_allowed: true,
            cop_count: 1,
            duration: 60.0,
            cops: vec![CopSlot { cop_type: COP_TYPE, speed_mph: 120.0, behaviour: 0, positioning: 0 }],
            ..Default::default()
        },
        spawn_distance: [350.0, 200.0, 400.0],
        remove_distance: [425.0, 250.0, 450.0],
        sight_chasing: 100.0,
        sight_idle: 50.0,
        sight_alert: 90.0,
        sight_near: 50.0,
        sight_near_slow: 50.0,
        sight_cone_deg: 40.0,
        sight_cone_alert_deg: 40.0,
        memory: 2.0,
        speeding_mph: if n == 0 { 110.0 } else { 300.0 },
        low_speed_mph: 40.0,
        search_speed_mph: 60.0,
        surplus_remove_distance: 100.0,
        crashed_remove_distance: 125.0,
        goto_timeout: 10.0,
        ..Default::default()
    }
}

fn config() -> PoliceConfig {
    let levels: Vec<HeatLevel> =
        [0.0, 2.0, 8.0, 16.0, 26.0, 38.0, 52.0].iter().enumerate().map(|(n, &t)| level(n as u8, t)).collect();
    PoliceConfig {
        radar_range: 200.0,
        bust_distance: 20.0,
        bust_speed_mph: 25.0,
        bust_time: 4.0,
        levels,
        chase_started_heat: 0.5,
        chase_resumed_heat: 0.5,
        spawn_weights: [10.0, 10.0, 10.0, 0.0],
        ..Default::default()
    }
}

fn cop_at(position: Vec3, forward: Vec3) -> CopState {
    CopState { id: 900, cop_type: COP_TYPE, position, forward, line_clear: true, on_screen: true, ..Default::default() }
}

fn player_at(position: Vec3, mph: f32) -> PlayerState {
    PlayerState {
        position,
        forward: Vec3::Z,
        velocity: Vec3::Z * mph * mw_police::MPH,
        speed_mph: mph,
        ..Default::default()
    }
}

fn tick(police: &mut Police, cfg: &PoliceConfig, player: PlayerState, cops: &[CopState]) -> Vec<PursuitEvent> {
    let input = PoliceInput {
        dt: DT,
        player,
        cops,
        road: RoadState::default(),
        mode: PursuitMode::default(),
        scoring_heat: 0.0,
        force_wanted: false,
    };
    police.tick(cfg, &input).events
}

/// A speeding player in a cop's cone: wanted, the cop chases, the chase starts with "CHASE INITIATED".
fn start_chase(police: &mut Police, cfg: &PoliceConfig) {
    let cop = cop_at(Vec3::new(0.0, 0.0, 40.0), Vec3::NEG_Z);
    let mut events = Vec::new();
    for _ in 0..10 {
        events.extend(tick(police, cfg, player_at(Vec3::ZERO, 120.0), &[cop]));
        if police.pursuit.state == PursuitState::Chase {
            break;
        }
    }
    assert_eq!(police.pursuit.state, PursuitState::Chase, "events {events:?}");
    assert!(events.contains(&PursuitEvent::Started));
    assert!(events.contains(&PursuitEvent::Score(ScoreSlot::ChaseSpotted)), "{events:?}");
    assert_eq!(police.cop(900).unwrap().role, Role::Chase);
    assert!((police.pursuit.heat - 0.5).abs() < 0.01, "chase initiated adds 0.5 heat: {}", police.pursuit.heat);
}

#[test]
fn slow_pass_is_no_pursuit() {
    let cfg = config();
    let mut police = Police::default();
    let cop = cop_at(Vec3::new(0.0, 0.0, 40.0), Vec3::NEG_Z);
    for _ in 0..120 {
        tick(&mut police, &cfg, player_at(Vec3::ZERO, 60.0), &[cop]);
    }
    assert_eq!(police.pursuit.state, PursuitState::None);
    assert_eq!(police.cop(900).unwrap().role, Role::Patrol);
}

#[test]
fn spotted_speeding_starts_a_chase() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    assert_eq!(police.pursuit.level_number(&cfg), 1);
}

#[test]
fn out_of_sight_cools_down_and_escapes() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    // The player drives off at 50 m/s with the line blocked; the cop stays.
    let mut cop = cop_at(Vec3::new(0.0, 0.0, 40.0), Vec3::NEG_Z);
    cop.line_clear = false;
    let mut events = Vec::new();
    let mut z = 0.0;
    for _ in 0..(60 * 30) {
        z -= 50.0 * DT;
        events.extend(tick(&mut police, &cfg, player_at(Vec3::new(0.0, 0.0, z), 110.0), &[cop]));
        if police.pursuit.state == PursuitState::Escaped {
            break;
        }
    }
    let at = |e: &PursuitEvent| events.iter().position(|x| x == e);
    let cooldown = at(&PursuitEvent::Cooldown).expect("cooldown");
    assert!(at(&PursuitEvent::Score(ScoreSlot::Cooldown)).is_some_and(|k| k > cooldown));
    let escaped = at(&PursuitEvent::Escaped).expect("escaped");
    assert!(escaped > cooldown);
    assert!(events.contains(&PursuitEvent::Score(ScoreSlot::Escaped)));
    assert!(police.pursuit.heat < 0.05, "the cooldown cooled the heat: {}", police.pursuit.heat);
    // The cop lost the player: it searched or was sent where the player was.
    assert_ne!(police.cop(900).unwrap().role, Role::Chase);
}

#[test]
fn stopped_near_a_cop_is_busted() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    let cop = cop_at(Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    let mut events = Vec::new();
    let mut t = 0.0;
    while t < 6.0 && police.pursuit.state == PursuitState::Chase {
        events.extend(tick(&mut police, &cfg, player_at(Vec3::ZERO, 0.0), &[cop]));
        t += DT;
    }
    assert_eq!(police.pursuit.state, PursuitState::Busted, "{events:?}");
    assert!(events.iter().any(|e| matches!(e, PursuitEvent::BustCaught { cop: 0 })));
    // The meter takes the bust time (4 s) to fill.
    assert!((3.9..4.3).contains(&t), "busted after {t:.2} s");
    assert!(police.pursuit.show_bust_bar);
    // The cop switched to the low-speed chase near the stopped player.
    assert_eq!(police.cop(900).unwrap().driver.state_id(), 0x16);
}

#[test]
fn moving_away_drains_the_bust_meter() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    let cop = cop_at(Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    for _ in 0..120 {
        tick(&mut police, &cfg, player_at(Vec3::ZERO, 0.0), &[cop]);
    }
    let filled = police.pursuit.bust_meter;
    assert!((0.45..0.55).contains(&filled), "2 s of 4: {filled}");
    for _ in 0..60 {
        tick(&mut police, &cfg, player_at(Vec3::ZERO, 30.0), &[cop]);
    }
    assert!(police.pursuit.bust_meter < filled - 0.2);
    assert_eq!(police.pursuit.state, PursuitState::Chase);
}

#[test]
fn heat_rises_in_a_chase_and_levels_up() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    // Level 1: threshold 2 over 350 s.
    let cop = cop_at(Vec3::new(0.0, 0.0, 40.0), Vec3::NEG_Z);
    let before = police.pursuit.heat;
    for _ in 0..600 {
        tick(&mut police, &cfg, player_at(Vec3::ZERO, 120.0), &[cop]);
    }
    let rate = (police.pursuit.heat - before) / 10.0;
    assert!((rate - 2.0 / 350.0).abs() < 1e-3, "heat rate {rate}");
    // Heat from scoring actions (takedowns, SLAM …) pushes it over the threshold: level 2 and its message.
    let input = PoliceInput {
        dt: DT,
        player: player_at(Vec3::ZERO, 120.0),
        cops: &[cop],
        road: RoadState::default(),
        mode: PursuitMode::default(),
        scoring_heat: 2.0,
        force_wanted: false,
    };
    let events = police.tick(&cfg, &input).events;
    assert!(events.contains(&PursuitEvent::LevelUp(2)), "{events:?}");
    assert!(events.contains(&PursuitEvent::Score(ScoreSlot::HeatUp)));
    // The level's own action the first time it is reached.
    let events = tick(&mut police, &cfg, player_at(Vec3::ZERO, 120.0), &[cop]);
    assert!(events.contains(&PursuitEvent::Score(ScoreSlot::Level(5002))), "{events:?}");
    assert_eq!(police.pursuit.max_level, 2);
}

#[test]
fn heat_is_capped_below_the_last_unlocked_level() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    let mode = PursuitMode { unlocked_levels: Some(3), ..Default::default() };
    let mut ev = Vec::new();
    police.pursuit.add_heat(&cfg, &mode, 100.0, &mut ev);
    assert!((police.pursuit.heat - (8.0 - 0.001)).abs() < 1e-4, "{}", police.pursuit.heat);
}

#[test]
fn a_roadblock_ahead_resumes_the_cooldown() {
    let cfg = config();
    let mut police = Police::default();
    start_chase(&mut police, &cfg);
    police.pursuit.state = PursuitState::Cooldown;
    police.pursuit.time_til_evaded = 5.0;
    let input = PoliceInput {
        dt: DT,
        player: player_at(Vec3::new(0.0, 0.0, -500.0), 80.0),
        cops: &[],
        road: RoadState { roadblocks: &[Vec3::new(0.0, 0.0, -450.0)], ..Default::default() },
        mode: PursuitMode::default(),
        scoring_heat: 0.0,
        force_wanted: false,
    };
    let events = police.tick(&cfg, &input).events;
    assert!(events.contains(&PursuitEvent::Resumed), "{events:?}");
    assert!(events.contains(&PursuitEvent::Score(ScoreSlot::ChaseResumed)));
    assert_eq!(police.pursuit.state, PursuitState::Chase);
}

#[test]
fn coordinator_orders_the_levels_cops() {
    let cfg = config();
    let mut police = Police::default();
    let input = PoliceInput {
        dt: DT,
        player: player_at(Vec3::ZERO, 30.0),
        cops: &[],
        road: RoadState::default(),
        mode: PursuitMode::default(),
        scoring_heat: 0.0,
        force_wanted: false,
    };
    let out = police.tick(&cfg, &input);
    assert_eq!(out.spawns.len(), 1);
    let s = out.spawns[0];
    assert_eq!(s.slot.cop_type, COP_TYPE);
    assert_eq!(s.distance, 350.0);
    assert_eq!(s.kind, mw_police::coordinator::SpawnKind::Generic);
    // The order counts: nothing more while it is pending.
    for _ in 0..120 {
        assert!(police.tick(&cfg, &input).spawns.is_empty());
    }
}

#[test]
fn sequencer_cycles_and_restarts_per_level() {
    let mut l = level(2, 8.0);
    l.main.duration = 10.0;
    let mut cycle = l.main.clone();
    cycle.roadblock = 77;
    cycle.duration = 5.0;
    l.cycle = vec![cycle];
    let mut s = Sequencer::default();
    s.tick(&l, 0.1);
    assert_eq!(s.behaviour(&l).roadblock, 0);
    for _ in 0..100 {
        s.tick(&l, 0.1);
    }
    assert_eq!(s.behaviour(&l).roadblock, 77, "the cycle behaviour after the main one's 10 s");
    for _ in 0..51 {
        s.tick(&l, 0.1);
    }
    assert_eq!(s.behaviour(&l).roadblock, 77, "a single cycle behaviour repeats");
}

#[test]
fn roadblock_triggers_ahead_on_the_route() {
    let mut l = level(2, 8.0);
    l.main.roadblock = 1234;
    l.main.roadblock_delay = 1.0;
    l.main.roadblock_repeat = 30.0;
    let seq = Sequencer::default();
    let route: Vec<Vec3> = (0..10).map(|k| Vec3::new(0.0, 0.0, 100.0 * k as f32)).collect();
    let triggers = [
        TriggerRegion { position: Vec3::new(5.0, 0.0, 150.0), radius: 20.0 },
        TriggerRegion { position: Vec3::new(5.0, 0.0, 450.0), radius: 20.0 },
    ];
    let mut task = RoadblockTask::default();
    let q = RoadblockQuery { chasing: true, player: Vec3::ZERO, route: &route, triggers: &triggers };
    assert_eq!(task.tick(0.5, &seq, &l, &q), None, "before the first delay");
    assert_eq!(task.tick(0.6, &seq, &l, &q), Some((1, 1234)));
    assert!((task.timer - 30.0).abs() < 1e-3, "the repeat time");
}
