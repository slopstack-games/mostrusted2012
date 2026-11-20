//! The logic's race and game-mode requests carried out in the runtime (PLAN M8.7; a child module of `logic.rs`,
//! which hands every request it does not take itself to [`RuntimeHost::race_request`]). Each request names the
//! original's function its native calls (`mw_logic::natives::Request`'s doc); what that function changes is done
//! here, on the runtime's system that owns it, and the hand-written trigger it replaces is gone:
//!
//! - `AddAllAIPlayers` (`FUN_00686f30`): the event's racers take the player slots 12.. at once (the hook
//!   `AreAllAIPlayersLoaded` reads them right after; `race.rs` marks a slot loaded once its car is placed), and
//!   `ai.rs` builds the grid and requests the cars ([`RaceDriven::add_ai`], before: as soon as the race existed).
//!   `SetIntroLoadComplete` then sets `Mode.HasIntroLoadCompleted` (the former stand-in in `RaceLink` is gone).
//! - `PauseAi` (`PAUSEAI`, G+0x2ef50..0x2ef5f): the racer state (`FUN_0050da90`) returns before driving while the
//!   car's byte is set (`FUN_006a05d0`): `ai::AiPaused`.
//! - `GameModeTimer` (`FUN_006f0f80` / `FUN_006f1120`): the race clock starts ([`RaceDriven::timer`]; before: at the
//!   intro's event trigger). The event's `Timeline` sequences it starts are `Request::PlayEventTimeline`
//!   (`event_timeline.rs`); the timeout it arms is the logic's (GMM+0x2f18, `Query::DisplayTimeLeft`).
//! - `ResetVehicleDamage` / `CleanAndRepairVehicle` (message 0x16 → `FUN_0058f060`): the local car's damage zeroed
//!   and the car washed (physics message 0x12), as the body shop's repair.
//! - `ResetProps` (message 0x15 → `FUN_0055a680` case 0x15: `FUN_00559b50`, `FUN_00559e20`, `FUN_005536f0`): every
//!   knocked prop is put back at once (`props::ResetAllProps`).
//! - `RestartTraffic` (message 7 → `FUN_0060a380` case 7, offline `FUN_00615020`): the traffic restarts (every
//!   car goes, the stop flag +0xa2b4c cleared): `Carry::Traffic(7)`.
//! - `AdjustScoringDisabled` (scoring manager +0x124, read by `FUN_006a75f0`): no scoring action awards while it
//!   is above 0 (`freeroam::ScoringDisabled`).
//! - `Respawn` of no event (`RequestRespawn`, `FUN_006c9fb0(0, 0)` → `FUN_006d0180`): [`respawn_in_place`]; of the
//!   free-drive event: the start-up placement (`frontend.rs`) already put the car there (model decision).
//! - `StopAllSequences` (`FUN_00a44dd0` on the players UIM+0x47748 / +0x4774c): the playbacks on players 0 and 1
//!   stop.
//!
//! Carried out with nothing for the runtime to do (the reason with each): `StartGameMode` (the GMM start is the
//! logic's own, `mode_machine::start_game_mode`; its message 5 (`FUN_00846710`) has no traced receiver),
//! `SetEndReason` (GMM+0x2d08, the logic's), `StartPlaying` (`FUN_006fa720` recounts the collectables, which
//! `freeroam.rs` counts at load; the session statistics are not kept), `EnterGame` (camera message 0 with profile
//! option 11 and three messages on the queue `DAT_010edb38` (2, 4, 8): the runtime keeps no saved camera choice;
//! the other queue's receiver was not traced), `AdjustTimeEnabled` (G+0x25624: only its four writers exist in the
//! code, no reader), `StreamVehicle` / `ReleaseVehicle` (the loader reads a car's bundle when the car is built;
//! `Query::VehicleStreamBusy` stays false), `ResetCurrentEventStats` (the pursuit / event statistics managers are
//! not in the runtime; a race's progress restarts at go), `WeaponsAllowed` / `ResetPlayerWeapons` (the player
//! weapon is the spike strip (`SpikeStripWeapon`), gated by `WEAPONSOK` (event byte +0xf2) and WEAPON_DEFENCE; no
//! offline event allows it and the runtime has no player weapon: the answer with none equipped), `StartCollectables`
//! (`FUN_0070d260`'s counts and found marks are `freeroam.rs`'s load, done before `Query::CollectablesReady` lets
//! the request come), `BlacklistStep` (no rival roams free roam: the spawn `FUN_00748a40` is not ported, so the
//! player manager's rival slot +0xa698 stays empty and the phase stays; `Query::BlacklistPhaseAfter` 0).

use bevy::prelude::*;
use mw_logic::natives::players::{FIRST_AI_SLOT, Player, SLOTS};
use mw_logic::natives::{Request, mode_machine};

use super::{Carry, RuntimeHost};
use crate::race::RaceDriven;

/// What `tick` carries out on the runtime's systems for these requests ([`RaceFx::apply`]).
#[derive(Debug)]
pub(super) enum RaceCarry {
    AddAiPlayers,
    PauseAi(bool),
    ModeTimer(bool),
    /// Message 0x16: repair and wash.
    Repair,
    ResetProps,
    ScoringDisabled(i32),
    /// `FUN_006c9fb0(0, 0)`; the mode timer runs (the rolling reset speed).
    Respawn {
        timer_running: bool,
    },
}

impl RuntimeHost {
    /// The race / game-mode requests (module doc): `Some` when carried out (or nothing is to be done), `None` when
    /// the request is not one of these.
    pub(super) fn race_request(&mut self, r: &Request) -> Option<Option<Carry>> {
        let race = |c| Some(Some(Carry::Race(c)));
        let nothing = Some(Some(Carry::Nothing));
        match r {
            Request::AddAllAIPlayers => {
                let mut st = self.logic.state.borrow_mut();
                let current = st.game_modes.current_event;
                let Some(setup) = self.race.setup.as_ref().filter(|s| s.event.id == current) else {
                    return nothing;
                };
                // `FUN_00686f30`: one player per racing `AIPlayerInformation` from slot 12 (`FUN_006ce030`), without
                // the slot +0xee leaves out. Model decision: cops are left out as `race.rs`'s grid leaves them out.
                let e = &setup.event;
                let racers = e.opponents.iter().filter(|o| !o.cop && o.start.is_some()).count();
                let racers = racers - usize::from(e.skip_player_car_ai && racers > 0);
                for k in 0..racers.min(SLOTS - FIRST_AI_SLOT) {
                    let slot = FIRST_AI_SLOT + k;
                    st.players.slots[slot] = Some(Player { kind: 2, vehicle: slot as i32, ..Default::default() });
                }
                info!("hsm: {:5} AddAllAIPlayers: {racers} racers in slots {FIRST_AI_SLOT}..", self.frame);
                race(RaceCarry::AddAiPlayers)
            }
            Request::PauseAi { paused } => race(RaceCarry::PauseAi(*paused)),
            Request::GameModeTimer { running } => race(RaceCarry::ModeTimer(*running)),
            Request::ResetVehicleDamage { vehicle } | Request::CleanAndRepairVehicle { vehicle } => {
                let local = self.logic.state.borrow().players.local_player().map(|p| p.vehicle);
                if *vehicle == -1 || Some(*vehicle) != local {
                    return nothing;
                }
                race(RaceCarry::Repair)
            }
            Request::ResetProps => race(RaceCarry::ResetProps),
            Request::RestartTraffic => Some(Some(Carry::Traffic(7))),
            Request::AdjustScoringDisabled { delta } => race(RaceCarry::ScoringDisabled(*delta)),
            // `FUN_006c9fb0` returns at once while a reset is under way (player +0x2c1): the respawn at the event's
            // start still placing the car (`logic_effects::place_vehicles`).
            Request::Respawn { event: 0, to_swap_spot: false }
                if self.logic.state.borrow().players.local_player().is_some_and(|p| p.reset_pending) =>
            {
                info!("hsm: {:5} respawn ignored: a reset is under way", self.frame);
                nothing
            }
            Request::Respawn { event: 0, to_swap_spot: false } => {
                let timer_running = self.logic.state.borrow().game_modes.timer_running;
                race(RaceCarry::Respawn { timer_running })
            }
            // `RequestRespawnAtEventStart` of the free-drive event (`FUN_006cd380`): the free-drive start is where the
            // runtime's start-up placement put the car (`frontend.rs`, before the logic runs; model decision: the
            // placement is not moved onto this request).
            Request::Respawn { event, to_swap_spot: false }
                if mode_machine::is_freedrive_event(&self.logic.state.borrow(), *event) =>
            {
                nothing
            }
            // `RequestRespawnAtEventStart` of an event `race.rs` does not start (the game's intro 1560354 "WELCOME TO
            // FAIRHAVEN" and the other non-race modes): `FUN_006cd380` → `FUN_006c9fb0` → `FUN_006d0180`, the car on
            // the event's spawn point (`SpawnPosition` +0x9c, resolved from G+0x2f6f8) facing its direction, at rest
            // (the mode timer does not run yet), placed by `FUN_00647e00` (reason 6) once the world there is in.
            Request::Respawn { event, to_swap_spot: false } if *event != 0 && *event != self.race.event() => {
                let (position, direction) = *self.event_starts.get(event)?;
                info!("hsm: {:5} respawn at the start of event {event}: {position:.1?}", self.frame);
                Some(Some(Carry::Effect(crate::logic_effects::Effect::Place(crate::logic_effects::Placement {
                    position,
                    direction,
                    mph: 0.0,
                    reason: 6,
                }))))
            }
            Request::StopAllSequences => {
                self.playbacks.retain(|p| !matches!(p.player, Some(0 | 1)));
                nothing
            }
            // Owned by the logic, or with nothing in the runtime to change (module doc).
            Request::StartGameMode { .. }
            | Request::SetEndReason { .. }
            | Request::StartPlaying
            | Request::EnterGame
            | Request::AdjustTimeEnabled { .. }
            | Request::StreamVehicle { .. }
            | Request::ReleaseVehicle { .. }
            | Request::ResetCurrentEventStats { .. }
            | Request::WeaponsAllowed(_)
            | Request::ResetPlayerWeapons
            | Request::StartCollectables
            | Request::BlacklistStep { .. } => nothing,
            _ => None,
        }
    }
}

/// The runtime's systems the race requests change.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct RaceFx<'w, 's> {
    ai_paused: Option<ResMut<'w, crate::ai::AiPaused>>,
    props: Option<ResMut<'w, crate::props::ResetAllProps>>,
    scoring: Option<ResMut<'w, crate::freeroam::ScoringDisabled>>,
    respawn: Option<ResMut<'w, PendingRespawn>>,
    timeline: Option<ResMut<'w, crate::event_timeline::EventTimeline>>,
    #[allow(clippy::type_complexity)]
    cars: Query<
        'w,
        's,
        (Option<&'static mut crate::damage::CarDamage>, Option<&'static mut crate::paint_wear::CarGrime>),
        With<crate::drive::Player>,
    >,
}

impl RaceFx<'_, '_> {
    /// The event's `Timeline` sequences (`event_timeline.rs`).
    pub(super) fn timeline(&mut self) -> Option<&mut crate::event_timeline::EventTimeline> {
        self.timeline.as_deref_mut()
    }

    /// Carry one request out.
    pub(super) fn apply(&mut self, c: RaceCarry, driven: Option<&mut RaceDriven>) {
        match c {
            RaceCarry::AddAiPlayers => {
                if let Some(d) = driven {
                    d.add_ai = true;
                }
            }
            RaceCarry::ModeTimer(running) => {
                if let Some(d) = driven {
                    d.timer = running;
                }
            }
            RaceCarry::PauseAi(paused) => {
                if let Some(p) = self.ai_paused.as_deref_mut() {
                    p.0 = paused;
                }
            }
            RaceCarry::Repair => {
                for (damage, grime) in &mut self.cars {
                    if let Some(mut d) = damage {
                        d.repair();
                    }
                    if let Some(mut g) = grime {
                        *g = Default::default();
                    }
                }
            }
            RaceCarry::ResetProps => {
                if let Some(p) = self.props.as_deref_mut() {
                    p.0 = true;
                }
            }
            RaceCarry::ScoringDisabled(delta) => {
                if let Some(s) = self.scoring.as_deref_mut() {
                    s.0 += delta;
                }
            }
            RaceCarry::Respawn { timer_running } => {
                if let Some(r) = self.respawn.as_deref_mut() {
                    r.0 = Some(timer_running);
                }
            }
        }
    }
}

/// The local player's pending reset (+0x2c2, set by `FUN_006c9fb0`): `Some(the mode timer runs)`.
#[derive(Resource, Default)]
pub struct PendingRespawn(pub Option<bool>);

/// The reset `FUN_006d0180` runs for a pending `RequestRespawn`: the car goes back to its saved pose (model
/// decision: where it stands now), facing the race's target gate when the event has a route (`FUN_00658310`:
/// `race.rs`'s respawn facing, before go the first gate; else its own heading), at 0 mph unless the mode timer
/// runs (then max(`GameLogicConfig` +0x220, speed × +0x224), the rolling reset), through `FUN_00647e00` (reason
/// 6). The target is the car's own spot, Resident already, so it is placed at once (`FUN_0064b290` places on
/// residency; `logic_effects::place_vehicles`'s extra ground test would wait for the collision under it).
#[allow(clippy::type_complexity)]
pub(crate) fn respawn_in_place(
    pending: Option<ResMut<PendingRespawn>>,
    facing: Option<Res<crate::race::RespawnFacing>>,
    race: Option<Res<crate::race::RaceState>>,
    crash: Option<Res<crate::crash::CrashConfig>>,
    mut cars: Query<
        (
            &mut Transform,
            &mut avian3d::prelude::LinearVelocity,
            &mut avian3d::prelude::AngularVelocity,
            &mut crate::drive::Car,
        ),
        With<crate::drive::Player>,
    >,
) {
    let Some(mut pending) = pending else { return };
    let Some(timer_running) = pending.0 else { return };
    let Ok((mut t, mut lin, mut ang, mut car)) = cars.single_mut() else { return };
    pending.0 = None;
    let position = t.translation;
    let gate =
        facing.as_ref().and_then(|f| f.0).or_else(|| race.as_ref().and_then(|r| r.entries.get(1)).map(|e| e.position));
    let direction = gate.map(|g| (g - position).with_y(0.0)).filter(|d| d.length() > 0.1);
    let direction = direction.unwrap_or(t.rotation * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z);
    let mph = match (&crash, timer_running) {
        (Some(cfg), true) => cfg.0.reset.after_crash(lin.0.length() * 2.236_94),
        _ => 0.0,
    };
    let speed = mph * 0.447_04;
    let yaw = Quat::from_rotation_y(direction.x.atan2(direction.z));
    t.rotation = yaw;
    lin.0 = yaw * Vec3::Z * speed;
    ang.0 = Vec3::ZERO;
    car.place_moving(speed);
    info!("logic: respawn in place at {position:.1?} facing {direction:.2?}, {mph:.0} mph (reason 6)");
}
