//! The player manager **PM** (G+0x2f3dc) as far as the HSM hooks read and write it: 24 player records of 0x7f0 B
//! inline from +0x10, a pointer per slot at +0xa5d0 (null = empty; it points at the slot's own record, read in
//! the snapshot), the local player's slot at +0xa6b4 (−1 = none; `FUN_006bf330` returns its record). Slots 12..23
//! are the AI racers (decomp.md "Races").

/// Number of player slots.
pub const SLOTS: usize = 24;
/// The first AI racer slot.
pub const FIRST_AI_SLOT: usize = 12;

/// One player record (offsets in the record).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Player {
    /// +0x50: the competitor name (a wide string pointer; `GetCompetitorName`).
    pub name: String,
    /// +0x268: the team (`GetCompetitorTeam`).
    pub team: i32,
    /// +0x9c: the kind (2 = an AI-driven car; `FUN_006bf190` reads the cop flag only for those).
    pub kind: u32,
    /// (+0x578)+0xb5: the car is a cop (`FUN_006bf190`).
    pub cop: bool,
    /// `FUN_006ba2f0`: the car is loaded (engine state; the runtime sets it).
    pub loaded: bool,
    /// +0xa0: the vehicle's slot in the world (−1 = none).
    pub vehicle: i32,
    /// +0xac: the race target vehicle (`InvalidateRaceTarget` passes it to `FUN_00843170` unless it is −1).
    pub race_target: i32,
    /// +0x28c / +0x290: the car-still words `SetCarStill` writes (+0x28e = 1 sets the car still, false clears both).
    pub car_still: u32,
    pub car_still_290: bool,
    /// `FUN_006ba2a0` → `FUN_00573390`(vehicle): the vehicle exists in the world (engine state).
    pub vehicle_alive: bool,
    /// +0x3d8, +0x3dc, +0x3e8: the heat values the hooks zero (+0x3dc is the vehicle's heat: `TransferVehicleHeatToPlayer`).
    pub heat_3d8: f32,
    pub heat_3dc: f32,
    pub heat_3e8: f32,
    /// +0x2c4: the respawn's target event (0 = where the player is).
    pub respawn_event: u32,
    /// +0x435: respawn at the last swap spot.
    pub respawn_to_swap_spot: bool,
    /// +0x418: the event score (`IsTargetScoreHit`).
    pub score: i32,
    /// +0x5b8, +0x5c8, +0x5cc, +0x5d0: the final event data (`SetFinalEventData`, `FUN_006b9d00`).
    pub final_5b8: i32,
    pub final_5c8: u32,
    pub final_5cc: u32,
    pub final_5d0: bool,
    /// +0x3fc: the XP the player earned online and has not uploaded yet (`UploadLocalPlayerXP` zeroes it).
    pub online_xp: i32,
    /// `FUN_006ba320` (the player's vtable +0x18): the `VehicleList` id of its car (engine state; the runtime
    /// sets it; 0 = none). `Mode.CurrentVehicleGamepack` reads its pack.
    pub vehicle_id: u32,
    /// +0x23d `IsActive`, +0x23e `Wrecked`, +0x23f `Crashed` (fields of the `Players` binding root,
    /// [`players_view`](super::players_view)).
    pub active: bool,
    pub wrecked: bool,
    pub crashed: bool,
    /// +0x434 `WasBusted`.
    pub was_busted: bool,
    /// +0x2c8 `IsTeleporting`.
    pub teleporting: bool,
    /// +0x2f6 `IsFallingOffCliff`.
    pub falling_off_cliff: bool,
    /// +0x285 `IsInBuilding`.
    pub in_building: bool,
    /// +0x474 `IsMidGameJoining`.
    pub mid_game_joining: bool,
    /// +0x2c1: a respawn is under way (`FUN_006c9fb0` sets it, the reset's end clears it; engine state, the
    /// runtime owns it). `ResetInProgress` also holds while the vehicle's flag G+0x25694[vehicle] does
    /// ([`Self::vehicle_resetting`]).
    pub reset_pending: bool,
    /// G+0x25694 + `vehicle`: the world's per-vehicle reset flag (engine state; the runtime sets it).
    pub vehicle_resetting: bool,
    /// +0x2ea / +0x2e8: the wreck's cause is known / it is a suicide (`IsWreckSuicide`).
    pub wreck_cause_known: bool,
    pub wreck_suicide: bool,
    /// `IsEngineOn` (`FUN_006d8ab0` -> `FUN_00572c20` on the world's vehicle; engine state, the runtime sets it).
    pub engine_on: bool,
    /// `IsInWaitForZone` (`FUN_00515120` on `*0x010edb50`'s record of the player; engine state).
    pub in_wait_for_zone: bool,
    /// `HasChaseStarted` (0x6db740: the pursuit record G+0x2f410 + slot x 0x8b0, +0x89c != 0; the pursuit
    /// system's state, the runtime sets it).
    pub chase_started: bool,
    /// `CurrentNitrousType` (`FUN_006bde60`): the vehicle gameplay record's (+0x84 class, +0x80 kind) behind the
    /// player's vtable +0x18; `None` = no record (a null string). The runtime sets it from the car.
    pub nitrous: Option<(u32, u32)>,
}

/// The player manager.
#[derive(Debug, Clone)]
pub struct PlayerManager {
    /// +0xa5d0: the occupied slots.
    pub slots: [Option<Player>; SLOTS],
    /// +0xa6b4: the local player's slot.
    pub local: Option<usize>,
    /// +0xa6bc: the camera target's slot (None = the local player; `CameraTarget`).
    pub camera_target: Option<usize>,
    /// +0xa6ac `NumTeamPlayers` and +0xa6b0 `NumOpponents` (set by the game's match set-up; offline 0).
    pub team_players: i32,
    pub opponents: i32,
    /// +0xcbe9 `HasAnyPlayerFinishedCurrentEvent` (the runtime sets it when a car crosses the finish).
    pub any_finished: bool,
    /// +0xcba8: `SetCarFrozen`'s value (1.0 / 0.0).
    pub car_frozen: f32,
}

impl Default for PlayerManager {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            local: None,
            camera_target: None,
            team_players: 0,
            opponents: 0,
            any_finished: false,
            car_frozen: 0.0,
        }
    }
}

impl PlayerManager {
    /// The local player when its slot is occupied (+0xa6b4 ≠ −1 and +0xa5d0[slot] ≠ 0).
    pub fn local_player(&self) -> Option<&Player> {
        self.slots.get(self.local?)?.as_ref()
    }

    pub fn local_player_mut(&mut self) -> Option<&mut Player> {
        self.slots.get_mut(self.local?)?.as_mut()
    }
}
