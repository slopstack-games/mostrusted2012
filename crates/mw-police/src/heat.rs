//! Heat levels: which `HeatLevel` row a heat value is in, the band it spans and the cap (docs/decomp.md "Police and
//! pursuits", "The data"). The rows are `GameLogicConfig` +0xa0 (free roam: levels 0–6, thresholds 0 / 2 / 8 / 16 /
//! 26 / 38 / 52; `Threshold` is a `u16` at +0x10e, the heat at which the level *ends*).

use mw_assets::police::{HeatLevel, PoliceConfig};

/// `.rdata` 0x00f8bb34: the heat stops this far below the highest unlocked level's threshold (`FUN_006673a0`).
pub const HEAT_CAP_MARGIN: f32 = 0.001;

/// `FUN_00669640`: the level for a heat value: the first row with no heat outside a chase, else the first row from
/// the second on whose threshold lies above the heat (the last row when none does).
pub fn level_for(levels: &[HeatLevel], heat: f32, in_chase: bool) -> Option<&HeatLevel> {
    if heat == 0.0 && !in_chase {
        return levels.first();
    }
    levels.iter().skip(1).find(|l| heat < l.threshold).or(levels.last())
}

/// `FUN_00669860`: the row below the heat's level; its threshold is the lower bound of the heat's band.
pub fn level_below(levels: &[HeatLevel], heat: f32) -> Option<&HeatLevel> {
    let n = levels.len();
    let mut previous = levels.first();
    for l in levels.iter().take(n.saturating_sub(1)) {
        if heat < l.threshold {
            return previous;
        }
        previous = Some(l);
    }
    previous
}

/// The lower bound of the heat's band (`FUN_00669860`'s row's threshold).
pub fn band_low(levels: &[HeatLevel], heat: f32) -> f32 {
    level_below(levels, heat).map_or(0.0, |l| l.threshold)
}

/// `FUN_006673a0`: the heat cap: the threshold of the highest *unlocked* row (`FUN_0068f190`, the player's
/// progression) minus 0.001. `unlocked` = how many rows from the first are unlocked (`None`: all of them).
pub fn heat_cap(levels: &[HeatLevel], unlocked: Option<usize>) -> f32 {
    let n = unlocked.unwrap_or(levels.len()).min(levels.len());
    if n == 0 {
        return 0.0;
    }
    (levels[n - 1].threshold - HEAT_CAP_MARGIN).max(0.0)
}

/// `FUN_006bdc50`: progress through the heat's band, 0–1 (the fraction `Player.HeatLevelFloat` adds to the level
/// number, `Player.HeatProgress`).
pub fn band_progress(levels: &[HeatLevel], heat: f32, in_chase: bool) -> f32 {
    let Some(level) = level_for(levels, heat, in_chase) else { return 0.0 };
    let low = band_low(levels, heat);
    ((heat - low) / (level.threshold - low).max(1e-3)).clamp(0.0, 1.0)
}

/// `FUN_00669640` with the running event (GMM+0x2ce0, offline): an event with its own levels
/// (`PoliceConfig::event_levels`) gives its first row with no heat outside a chase, else its first row from the second
/// on whose threshold lies above the heat; a heat above all of them (or an event without levels) falls through to the
/// free-roam rows ([`level_for`]).
pub fn event_level_for(cfg: &PoliceConfig, heat: f32, in_chase: bool) -> Option<&HeatLevel> {
    let ev = &cfg.event_levels;
    if !ev.is_empty() {
        if heat == 0.0 && !in_chase {
            return ev.first();
        }
        if let Some(l) = ev.iter().skip(1).find(|l| heat < l.threshold) {
            return Some(l);
        }
    }
    level_for(&cfg.levels, heat, in_chase)
}

/// `FUN_0066aa30` (through `FUN_006bdc00`, the player's level row): the level a sequence forced (G+0x2f414 +0xa8),
/// else the heat's level ([`event_level_for`]).
pub fn level_row(cfg: &PoliceConfig, heat: f32, in_chase: bool) -> Option<&HeatLevel> {
    cfg.forced_level.as_ref().or_else(|| event_level_for(cfg, heat, in_chase))
}

/// `FUN_0066a7b0` (`Player.HeatLevelInt`): the forced level's number when that row has `forced_number` (+0x113),
/// else the heat's level's ([`event_level_for`]).
pub fn level_number(cfg: &PoliceConfig, heat: f32, in_chase: bool) -> u8 {
    match &cfg.forced_level {
        Some(l) if l.forced_number => l.level,
        _ => event_level_for(cfg, heat, in_chase).map_or(0, |l| l.level),
    }
}

/// `FUN_00669860` with the forced level and the running event's rows: the forced level when set, else the row below
/// the heat's level in the event's list when it has one, else in the free-roam list ([`level_below`]).
pub fn level_below_cfg(cfg: &PoliceConfig, heat: f32) -> Option<&HeatLevel> {
    if let Some(l) = &cfg.forced_level {
        return Some(l);
    }
    if !cfg.event_levels.is_empty() {
        return level_below(&cfg.event_levels, heat);
    }
    level_below(&cfg.levels, heat)
}

/// [`band_progress`] over the level row ([`level_row`]) and the row below ([`level_below_cfg`]) (`FUN_006bdc50`).
pub fn band_progress_cfg(cfg: &PoliceConfig, heat: f32, in_chase: bool) -> f32 {
    let Some(level) = level_row(cfg, heat, in_chase) else { return 0.0 };
    let low = level_below_cfg(cfg, heat).map_or(0.0, |l| l.threshold);
    ((heat - low) / (level.threshold - low).max(1e-3)).clamp(0.0, 1.0)
}
