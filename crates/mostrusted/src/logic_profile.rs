//! The logic's profile, progression and UI requests (logic.rs hands them here from its request match; research log
//! 2026-11-20 "Profile / progression / UI requests"). Each is carried out as the original's handler does, on the
//! runtime's own state, or answered as the offline PC game answers it:
//!
//! - `NewGame` (`FUN_0068fd50`): a new game empties the progression tables (`FUN_0072bf20`; the logic's
//!   `last_played` in `hooks_flow::set_new_game`, here the profile's collectables "Collect3" / "CollectTypes": the
//!   jack spots, body shops and swap spot of [`Collected`], the free-roam finds and the Speed Points bucket
//!   "SPBucket" of `freeroam::Collectables`), then saved. A continued game's saved position (`FUN_00847780`) is
//!   where the start-up placed the car (main.rs, `StartConfig::continue_pose`; the world loads around it before
//!   the logic runs): nothing more to do.
//! - `SaveProfile` (`FUN_006f90c0` + `FUN_00733c70` and the save): our profile file is written now
//!   (`frontend::save_profile`), with the logic's `last_played` table; the logic's "Regular Autosave" (a
//!   `SaveProfile` every 120 s) replaces the runtime's own 10 s timer.
//! - `SetRolloutFromProfile` (`FUN_00692550`): the car of the profile's rollout ([`ProfileCar`]: the last car
//!   driven outside an event) is driven when another car is (`GarageAction::Drive`, its first paint: the
//!   runtime's profile keeps no paint).
//! - `PopulateSettings` (`FUN_0086e0a0`): the four settings pages' refresh (vtable +4 of SM+4, +0x854, +0x6c4,
//!   +0x7b0). The page classes are not ported (`lua_settings.rs`): the runtime's settings are `settings.cfg`'s,
//!   read with the profile (`LoadProfile`) and applied by `ApplyProfileSettings`; nothing else to refresh.
//! - `UpdateBodyShop` (`DOBODYSHOP` update 0x749220): `bodyshop.rs`'s pass runs on the frames the behaviour
//!   updates ([`BodyShopUpdate`]), not on its own.
//! - `UpdateMinimapIcons` (`MINIMAPICONS` update 0x7579b0): the players' minimap icons are kept by the behaviour
//!   (the slots' visible byte cleared, set again for each player that passes `FUN_0074e0b0`); the HUD's minimap
//!   draws the other players' icons on the frames it updates ([`MinimapIconsUpdate`]).
//! - `ClearSelectedEvent` (`CLRSELEVENT` 0x73baa0): the world map's selection is cleared
//!   (`map_screen::MapScreen::clear_selected_event`).
//! - `SetBlackBackground` (`SETBLACKBACK` 0x745b10, UIM+0x2d260): `FUN_006a0310` (read by the renderer's frame
//!   setup `FUN_0061f800`) then skips the world. The boot sets it ("Top State" 21597) and "InGame" clears it;
//!   the runtime's front-end screens (title, loading) cover the frame meanwhile; `FrontEnd::black_background`.
//! - `SetProfileSyncFlag` (`SYNCPROFILEW` finish): G+0x2ee5c = 1, answered as `Query::ProfileSyncFlag`.
//!
//! Offline answers (the PC game without a connection, as `hsm/behaviours_online.rs`):
//! - `GetSpeedwallTimes` (`FUN_006f1dc0`): the speed-wall leaderboard asks Autolog for the friends' times; no
//!   request goes out, so none is pending (`Query::SpeedwallIdle` stays true).
//! - `RefreshRecommendations` (`FUN_007355b0`): the "Recommends" document comes from the online service; none
//!   answers, so the cached index stays −1.
//! - `UpdateCachedMostWantedList` (`FUN_0071dfe0`): the notification centre's "GetMostWantedList" request
//!   (`FUN_00823b20`) fails without a server: the request id stays −1.
//! - `ServerOpStart` / `ServerOpReport` / `ServerOpAck` (`HSM_SERV_OP`): no request is sent (the behaviour's
//!   offline branch fails the operation over), no error is shown, nothing is acknowledged.
//! - `SetProfileUser` / `ProfileUserSelected` (`SELECT_PROF`): the profile named after the platform user's
//!   persona id and message 0xc; the runtime has one local profile file (`frontend::default_profile_path`).
//! - `ListenForPrimaryUser` / `ReleasePrimaryUser` (`HSB_P1BIND`): logic.rs's listener (the title's key press
//!   binds the user, `Query::PrimaryUserBound`).
//! - `SetCreditsTransition`: the logic's own field (GMM+0x2f26, `mode_machine::apply`).

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mw_logic::hsm::behaviours::Query as LogicQuery;
use mw_logic::natives::{LogicState, Request};

use crate::frontend::Collected;
use crate::garage::{Garage, GarageAction};

/// What logic.rs does with a request handled here.
pub enum Done {
    /// Carried out (or answered) with nothing left for the runtime's systems.
    Nothing,
    /// An action on the runtime's systems ([`Targets::apply`]).
    Act(Action),
}

/// An action on the runtime's systems.
#[derive(Debug)]
pub enum Action {
    /// `NewGame` of a new game: the profile's tables emptied.
    NewGame,
    /// `SaveProfile`: write the profile now, with the logic's `last_played` table.
    Save(BTreeMap<u8, i64>),
    /// `SetRolloutFromProfile`.
    RolloutFromProfile,
    UpdateBodyShop,
    UpdateMinimapIcons,
    ClearSelectedEvent,
    BlackBackground(bool),
}

/// The host's state for these requests.
#[derive(Default)]
pub struct State {
    /// G+0x2ee5c (`SYNCPROFILEW`).
    sync_flag: bool,
}

/// Handles `r` when it is one of this module's requests (`None` otherwise).
pub fn request(r: &Request, st: &LogicState, me: &mut State) -> Option<Done> {
    let act = |a| Some(Done::Act(a));
    match r {
        Request::NewGame { new_game: true, .. } => act(Action::NewGame),
        Request::NewGame { new_game: false, .. } => Some(Done::Nothing),
        Request::SaveProfile => act(Action::Save(st.progression.last_played.clone())),
        Request::SetRolloutFromProfile => act(Action::RolloutFromProfile),
        Request::UpdateBodyShop => act(Action::UpdateBodyShop),
        Request::UpdateMinimapIcons => act(Action::UpdateMinimapIcons),
        Request::ClearSelectedEvent => act(Action::ClearSelectedEvent),
        Request::SetBlackBackground(on) => act(Action::BlackBackground(*on)),
        // `FUN_006e65e0`: the GMM's post-event award (`FUN_00736a90` when the amount is above 0) and each collected
        // award (`FUN_00736ab0` / `FUN_006792b0`). With none (what `PreparePostEventAwards` leaves while
        // `FUN_006fbb60` is not ported) there is nothing to give; an award is left to the progression's port.
        Request::AwardPostEventAwards { amount, awards, .. } if *amount <= 0 && awards.is_empty() => {
            Some(Done::Nothing)
        }
        Request::SetProfileSyncFlag => {
            me.sync_flag = true;
            Some(Done::Nothing)
        }
        Request::PopulateSettings
        | Request::GetSpeedwallTimes { .. }
        | Request::RefreshRecommendations
        | Request::UpdateCachedMostWantedList
        | Request::ServerOpStart { .. }
        | Request::ServerOpReport { .. }
        | Request::ServerOpAck
        | Request::SetProfileUser { .. }
        | Request::ProfileUserSelected
        | Request::ListenForPrimaryUser { .. }
        | Request::ReleasePrimaryUser(_)
        | Request::SetCreditsTransition { .. } => Some(Done::Nothing),
        _ => None,
    }
}

/// The queries answered from this module's state.
pub fn query(q: &LogicQuery, me: &State) -> Option<u64> {
    match q {
        LogicQuery::ProfileSyncFlag => Some(me.sync_flag as u64),
        _ => None,
    }
}

/// `SaveProfile` under the logic (the logic's "Regular Autosave" decides when): read by `frontend::save_profile`.
#[derive(Resource, Default)]
pub struct SaveRequest {
    /// A save is asked for, with the logic's `last_played` table.
    pub pending: Option<BTreeMap<u8, i64>>,
}

/// `DOBODYSHOP`'s update ran this frame (`bodyshop::pass` runs on it). Present when the logic drives the free drive.
#[derive(Resource, Default)]
pub struct BodyShopUpdate {
    pub requested: bool,
}

/// `MINIMAPICONS`' update ran since the HUD last drew (`hud.rs` draws the other players' icons on it). Present when
/// the logic drives the free drive.
#[derive(Resource, Default)]
pub struct MinimapIconsUpdate {
    pub requested: bool,
}

/// The resources the requests drive (logic.rs's `LogicPlugin`; `direct`: no free drive is reached, the plain
/// managers keep their own timing).
pub fn build(app: &mut App, direct: bool) {
    if !direct {
        app.init_resource::<SaveRequest>()
            .init_resource::<BodyShopUpdate>()
            .init_resource::<MinimapIconsUpdate>()
            .init_resource::<ProfileCar>()
            .add_systems(Update, track_profile_car);
    }
}

/// The car of the profile's rollout: the car the player drives outside an event. Every car the runtime changes to
/// outside an event is the player's choice (a jack spot's swap, the car menus), which the original writes into the
/// profile's rollout (`FUN_00729f20`); an event's car is not.
#[derive(Resource, Default)]
pub struct ProfileCar(pub Option<String>);

fn track_profile_car(
    garage: Option<Res<Garage>>,
    race: Option<Res<crate::race::RaceState>>,
    mut car: ResMut<ProfileCar>,
) {
    let Some(garage) = garage else { return };
    if race.is_some() || garage.loading.is_some() {
        return;
    }
    if let Some(c) = &garage.current
        && car.0.as_deref() != Some(c.bundle.as_str())
    {
        car.0 = Some(c.bundle.clone());
    }
}

/// The systems the actions reach.
#[derive(SystemParam)]
pub struct Targets<'w, 's> {
    /// `FrontEnd` and `freeroam::Collectables` are also logic.rs's: changed through commands.
    commands: Commands<'w, 's>,
    save: Option<ResMut<'w, SaveRequest>>,
    collected: Option<ResMut<'w, Collected>>,
    garage: Option<Res<'w, Garage>>,
    actions: MessageWriter<'w, GarageAction>,
    body_shop: Option<ResMut<'w, BodyShopUpdate>>,
    minimap: Option<ResMut<'w, MinimapIconsUpdate>>,
    map: Option<ResMut<'w, crate::map_screen::MapScreen>>,
    profile_car: Option<Res<'w, ProfileCar>>,
}

impl Targets<'_, '_> {
    pub fn apply(&mut self, a: Action) {
        match a {
            Action::NewGame => {
                if let Some(c) = self.collected.as_deref_mut() {
                    // "SongLists" is not among the tables `FUN_0072bf20` empties.
                    let song_lists = std::mem::take(&mut c.song_lists);
                    *c = Collected { song_lists, dirty: true, ..Default::default() };
                }
                self.commands.queue(|w: &mut World| {
                    if let Some(mut c) = w.get_resource_mut::<crate::freeroam::Collectables>() {
                        c.reset_progress();
                    }
                });
            }
            Action::Save(last_played) => {
                if let Some(s) = self.save.as_deref_mut() {
                    s.pending = Some(last_played);
                }
            }
            Action::RolloutFromProfile => self.rollout_from_profile(),
            Action::UpdateBodyShop => {
                if let Some(b) = self.body_shop.as_deref_mut() {
                    b.requested = true;
                }
            }
            Action::UpdateMinimapIcons => {
                if let Some(m) = self.minimap.as_deref_mut() {
                    m.requested = true;
                }
            }
            Action::ClearSelectedEvent => {
                if let Some(m) = self.map.as_deref_mut() {
                    m.clear_selected_event();
                }
            }
            Action::BlackBackground(on) => {
                self.commands.queue(move |w: &mut World| {
                    if let Some(mut f) = w.get_resource_mut::<crate::frontend::FrontEnd>() {
                        f.black_background = on;
                    }
                });
            }
        }
    }

    /// `FUN_00692550`: the car of the profile's rollout ([`ProfileCar`]) is driven when another one is.
    fn rollout_from_profile(&mut self) {
        let (Some(profile), Some(garage)) = (self.profile_car.as_deref(), self.garage.as_deref()) else { return };
        let Some(bundle) = profile.0.clone() else { return };
        if garage.current.as_ref().is_some_and(|c| c.bundle == bundle) || garage.loading.is_some() {
            return;
        }
        let name = garage
            .catalogue
            .iter()
            .find(|e| e.bundle.eq_ignore_ascii_case(&bundle))
            .map_or_else(|| bundle.clone(), |e| e.name.clone());
        info!("hsm: SetRolloutFromProfile: the profile's car {bundle}");
        self.actions.write(GarageAction::Drive { name, bundle, paint_index: 0 });
    }
}
