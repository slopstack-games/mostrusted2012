//! `AIConfig` (`GLOBALCONFIG.BNDL` 010000000007fb0e), the fields the ported code reads, by instance offset
//! (the code reaches the instance through `DAT_010edac0` / `DAT_010edac4`). Defaults are the data's values.

/// One field: instance offset, GeneSys field hash, default (the install's value).
const FIELDS: &[(u16, u32, f32)] = &[
    (0x10, 0x54d23bfb, 10.0),
    (0x18, 0xbd6f7f45, 3.0),
    (0x28, 0x6f76d23f, 1.5),
    (0x44, 0x19ed104c, 500.0),
    (0xc8, 0x492f7ddb, 1.8),
    (0x124, 0x27d18b55, 30.0),
    (0x128, 0x63c7374a, 5.0),
    (0x140, 0xc064967b, 0.25),
    (0x144, 0xe9d45ba2, 15.0),
    (0x14c, 0xd695c3bb, 10.0),
    (0x154, 0x215239c6, 80.0),
    (0x158, 0x3102fe78, 20.0),
    (0x15c, 0x31558b83, 0.75),
    (0x160, 0x7aa16008, 0.5),
    (0x164, 0xef771e0c, 20.0),
    (0x168, 0x752104eb, 0.0),
    (0x16c, 0xec5e495d, 5.0),
    (0x170, 0x46fcb4e8, 25.0),
    (0x174, 0x67f69f2f, 80.0),
    (0x17c, 0xd2ff639f, 1.0),
    (0x180, 0x2b9adc03, 15.0),
    (0x184, 0xe24a12ac, 200.0),
    (0x188, 0x2c60106a, 0.2),
    (0x18c, 0x19085221, 90.0),
    (0x190, 0x40e55982, 5.0),
    (0x198, 0xfe4515b1, 0.15),
    (0x19c, 0x619a7200, 10.0),
    (0x1a0, 0x51c3d831, 10.0),
    (0x1a4, 0xae76a715, 3.0),
    (0x1cc, 0x96e5d35c, 20.0),
    (0x1d0, 0xcea544bb, 30.0),
    (0x1d8, 0x2287ac48, 1.5),
    (0x1dc, 0xdc079d1d, 60.0),
    (0x1e4, 0xa122ed3b, 2.0),
    (0x1e8, 0xa0ff77f2, 2.0),
    (0x1ec, 0x072ea6da, 8.0),
    (0x1f0, 0x8744d6b3, 0.6),
    (0x1f4, 0xc932b6d4, 90.0),
    (0x1f8, 0xa469edb2, 1.1),
    (0x1fc, 0xedfecd74, 0.45),
    (0x204, 0xa0a9c951, 1.0),
    (0x208, 0x4ba23c4a, 0.6),
];

/// The `AIConfig` values the racer code reads (field = instance offset; names are ours, from the use).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiConfig {
    /// 0x10: the AI's nitrous refill, tank units per second (`FUN_00667c10`, the AI branch; applied by the runtime).
    pub nitrous_refill: f32,
    /// 0x18: the lane offset's speed (m/s, `FUN_0050b000`).
    pub lane_rate: f32,
    /// 0x28: the emergency brake's time to contact (s) and search radius factor (`FUN_00508070`).
    pub contact_time: f32,
    /// 0x44: the obstacle sweep's longest length (m, `FUN_00503bf0`).
    pub sweep_max: f32,
    /// 0xc8: the corner speed rule's factor (`FUN_004ea860`; virtual progress only).
    pub corner_speed_scale: f32,
    /// 0x124 / 0x128: nitrous keeps firing below this turn ahead, starts below that (degrees, `FUN_004ff620`).
    pub nitrous_keep_turn: f32,
    pub nitrous_start_turn: f32,
    /// 0x140: tank needed to start nitrous.
    pub nitrous_min_tank: f32,
    /// 0x144: shortest look-ahead distance (m, `FUN_00506c00`).
    pub min_look: f32,
    /// 0x14c: nitrous starts only this far below the schedule speed (mph).
    pub nitrous_margin_mph: f32,
    /// 0x154 / 0x158: the racer interaction acts only from 0x158 m behind to 0x154 m ahead of the local player
    /// along the player's heading (`FUN_005050d0`).
    pub player_window_ahead: f32,
    pub player_window_behind: f32,
    /// 0x15c / 0x160: the shunt's longitudinal window, × 2 × the other car's +0x58 (ahead) / × −2 × (behind).
    pub shunt_ahead: f32,
    pub shunt_behind: f32,
    /// 0x164 / 0x168: mode 2's window: this car 0x168 m behind … 0x164 m ahead of the other along its heading.
    pub hold_ahead: f32,
    pub hold_behind: f32,
    /// 0x16c / 0x170: mode 2's speed difference window (own − other, mph): −0x170 … +0x16c.
    pub hold_dv_max_mph: f32,
    pub hold_dv_min_mph: f32,
    /// 0x174: mode 2 needs the other car at least this fast (mph).
    pub hold_min_mph: f32,
    /// 0x17c / 0x188: the shunt's random delay, lerp(0x188, 0x17c, r) seconds.
    pub shunt_delay_max: f32,
    pub shunt_delay_min: f32,
    /// 0x180: the shunt needs |own speed − other speed| ≤ this (mph).
    pub shunt_dv_mph: f32,
    /// 0x18c / 0x184: the interaction's own-speed window (mph).
    pub shunt_min_mph: f32,
    pub shunt_max_mph: f32,
    /// 0x190: the shunt's lateral reach beyond both radii (m).
    pub shunt_lateral: f32,
    /// 0x198: the shunt's steering per metre of lateral gap.
    pub shunt_steer: f32,
    /// 0x19c: no crash detection this close to a target point (states that pass one; not the racer).
    pub stuck_end_radius: f32,
    /// 0x1a0 / 0x1a4: reset after this long in the crash recovery (s; the second when not visible).
    pub reset_time: f32,
    pub reset_time_hidden: f32,
    /// 0x1cc: mode 2 ignores a car more than this many seconds away (gap / closing speed, `FUN_00501860`).
    pub hold_time_gap: f32,
    /// 0x1d0 / 0x1d8 / 0x1dc: mode 2's target speed (`FUN_004fda60`): at least 0x1d0 mph, 0x1d8 mph per metre of
    /// gap, within ±0x1dc mph of the other car's speed.
    pub hold_min_target_mph: f32,
    pub hold_gain_mph: f32,
    pub hold_window_mph: f32,
    /// 0x1e4 / 0x1e8: the obstacle sweep's time factors (s × speed; 0x1e4 for a player flagged by `FUN_006bf190`).
    pub sweep_time_a: f32,
    pub sweep_time_b: f32,
    /// 0x1ec: a node shortens the look-ahead when its inside clearance is below this (m, `FUN_00503880`).
    pub corner_clearance: f32,
    /// 0x1f0: the look-ahead scan time (s × speed metres) and the longest look-ahead time.
    pub look_scan_time: f32,
    /// 0x1f4: a node turning more than this (degrees) shortens the look-ahead.
    pub sharp_turn: f32,
    /// 0x1f8: the obstacle sweep's short length, s × speed.
    pub sweep_speed_factor: f32,
    /// 0x1fc: shortest look-ahead time (s).
    pub min_look_time: f32,
    /// 0x204 / 0x208: the obstacle sweep's box scales (the wide one brakes, the narrow one stops).
    pub sweep_wide: f32,
    pub sweep_narrow: f32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self::from_fields(|_| None)
    }
}

impl AiConfig {
    /// Build from the data: `field(hash)` returns the `AIConfig` instance's float field, `None` keeps the
    /// install's value.
    pub fn from_fields(field: impl Fn(u32) -> Option<f32>) -> Self {
        let v = |off: u16| {
            let &(_, hash, default) = FIELDS.iter().find(|f| f.0 == off).expect("listed offset");
            field(hash).unwrap_or(default)
        };
        Self {
            nitrous_refill: v(0x10),
            lane_rate: v(0x18),
            contact_time: v(0x28),
            sweep_max: v(0x44),
            corner_speed_scale: v(0xc8),
            nitrous_keep_turn: v(0x124),
            nitrous_start_turn: v(0x128),
            nitrous_min_tank: v(0x140),
            min_look: v(0x144),
            nitrous_margin_mph: v(0x14c),
            player_window_ahead: v(0x154),
            player_window_behind: v(0x158),
            shunt_ahead: v(0x15c),
            shunt_behind: v(0x160),
            hold_ahead: v(0x164),
            hold_behind: v(0x168),
            hold_dv_max_mph: v(0x16c),
            hold_dv_min_mph: v(0x170),
            hold_min_mph: v(0x174),
            shunt_delay_max: v(0x17c),
            shunt_dv_mph: v(0x180),
            shunt_max_mph: v(0x184),
            shunt_delay_min: v(0x188),
            shunt_min_mph: v(0x18c),
            shunt_lateral: v(0x190),
            shunt_steer: v(0x198),
            stuck_end_radius: v(0x19c),
            reset_time: v(0x1a0),
            reset_time_hidden: v(0x1a4),
            hold_time_gap: v(0x1cc),
            hold_min_target_mph: v(0x1d0),
            hold_gain_mph: v(0x1d8),
            hold_window_mph: v(0x1dc),
            sweep_time_a: v(0x1e4),
            sweep_time_b: v(0x1e8),
            corner_clearance: v(0x1ec),
            look_scan_time: v(0x1f0),
            sharp_turn: v(0x1f4),
            sweep_speed_factor: v(0x1f8),
            min_look_time: v(0x1fc),
            sweep_wide: v(0x204),
            sweep_narrow: v(0x208),
        }
    }

    /// The (offset, hash) pairs read, for tools that list them.
    pub fn fields() -> impl Iterator<Item = (u16, u32)> {
        FIELDS.iter().map(|f| (f.0, f.1))
    }
}
