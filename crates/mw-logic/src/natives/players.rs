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
    /// +0x9c: the kind (2 = an AI-driven car; `FUN_006bf190` reads the cop flag only for those).
    pub kind: u32,
    /// (+0x578)+0xb5: the car is a cop (`FUN_006bf190`).
    pub cop: bool,
    /// `FUN_006ba2f0`: the car is loaded (engine state; the runtime sets it).
    pub loaded: bool,
    /// +0xa0: the vehicle's slot in the world (−1 = none).
    pub vehicle: i32,
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
}

/// The player manager.
#[derive(Debug, Clone)]
pub struct PlayerManager {
    /// +0xa5d0: the occupied slots.
    pub slots: [Option<Player>; SLOTS],
    /// +0xa6b4: the local player's slot.
    pub local: Option<usize>,
}

impl Default for PlayerManager {
    fn default() -> Self {
        Self { slots: std::array::from_fn(|_| None), local: None }
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
