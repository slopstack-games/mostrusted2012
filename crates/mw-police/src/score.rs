//! The scoring actions the pursuit code posts (`FUN_006a8050`), by their `ScoringConfig` field (`GLOBALCONFIG.BNDL`
//! 390964, the object at `GlobalConfig` +0x44). The runtime resolves a slot to its `ScoringAction` through
//! `mw_assets::scoring` (the ids below are the shipped data's, for the docs and the tests); an action's Speed
//! Points, pursuit score, heat (`1a88cb4c`, +0x24, added by `FUN_006d08a0` → `FUN_006bdac0`), HUD message and
//! `GameplayTrigger` (radio, sequences) are the action's.

/// A `ScoringConfig` field that names a scoring action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScoreSlot {
    /// +0x2c (1289377): the player was wrecked by an AI car in a chase (`FUN_0065c880`).
    WreckedByAi,
    /// +0x48 (604071, "CHASE INITIATED", score 100, heat 0.5): the chase began at a roadblock (the player touched a
    /// roadblock car, `FUN_0065b930`), or the coordinator's per-player byte +0x448 was set.
    ChaseRoadblock,
    /// +0x4c (1264405): the chase began and the cop that knew the player is wrecked (its player +0x23e).
    ChaseCopWrecked,
    /// +0x50 (457827, "CHASE INITIATED", score 100, heat 0.5): the chase began because a cop knew the player.
    ChaseSpotted,
    /// +0x54 (955335, "CHASE INITIATED", score 25, heat 0.5): the chase began with no cop that knows the player (a
    /// takedown, a mode without a cooldown), or no chase-initiated action had been posted (`FUN_0065c5e0`).
    ChaseUnspotted,
    /// +0x58 (604021, heat 0.5, no title): a cooldown turned back into a chase (`FUN_0065c970`).
    ChaseResumed,
    /// +0x90 (1264470): the cops lost sight of the player while still chasing (`FUN_0065c740`).
    SightLost,
    /// +0x94 (1264469): the cops see the player again (`FUN_0065c740`).
    SightRegained,
    /// +0xc4 (390547, trigger 983054: the cooldown sequences): the cooldown began (`FUN_0065cbe0`).
    Cooldown,
    /// +0xe4 (955599, "HEAT LEVEL DECREASED"): `FUN_0065d160`.
    HeatDown,
    /// +0xe8 (390562, "HEAT LEVEL INCREASED"): `FUN_0065d080`.
    HeatUp,
    /// +0x1a8 (1377906): the player escaped (`FUN_0065ca70`).
    Escaped,
    /// A heat level's own action (`HeatLevel.a504e53b`, +0x6c), posted the first time the chase reaches the
    /// level (`FUN_0065c5e0`).
    Level(u32),
}

impl ScoreSlot {
    /// The field's offset in `ScoringConfig` (None for a heat level's own action).
    pub fn offset(self) -> Option<u32> {
        Some(match self {
            Self::WreckedByAi => 0x2c,
            Self::ChaseRoadblock => 0x48,
            Self::ChaseCopWrecked => 0x4c,
            Self::ChaseSpotted => 0x50,
            Self::ChaseUnspotted => 0x54,
            Self::ChaseResumed => 0x58,
            Self::SightLost => 0x90,
            Self::SightRegained => 0x94,
            Self::Cooldown => 0xc4,
            Self::HeatDown => 0xe4,
            Self::HeatUp => 0xe8,
            Self::Escaped => 0x1a8,
            Self::Level(_) => return None,
        })
    }

    /// The action id the shipped `ScoringConfig` holds in the field (`mwtool genesys 010000000005f734`).
    pub fn shipped_action(self) -> u32 {
        match self {
            Self::WreckedByAi => 1289377,
            Self::ChaseRoadblock => 604071,
            Self::ChaseCopWrecked => 1264405,
            Self::ChaseSpotted => 457827,
            Self::ChaseUnspotted => 955335,
            Self::ChaseResumed => 604021,
            Self::SightLost => 1264470,
            Self::SightRegained => 1264469,
            Self::Cooldown => 390547,
            Self::HeatDown => 955599,
            Self::HeatUp => 390562,
            Self::Escaped => 1377906,
            Self::Level(id) => id,
        }
    }

    /// One of the four "chase initiated" fields (`FUN_006691d0` sets the chase record's +0x8a9 for them).
    pub fn is_chase_initiated(self) -> bool {
        matches!(self, Self::ChaseRoadblock | Self::ChaseCopWrecked | Self::ChaseSpotted | Self::ChaseUnspotted)
    }
}
