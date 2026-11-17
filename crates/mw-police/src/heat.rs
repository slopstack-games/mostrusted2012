//! Heat levels: which `HeatLevel` row a heat value is in, the band it spans and the cap (docs/decomp.md "Police and
//! pursuits", "The data"). The rows are `GameLogicConfig` +0xa0 (free roam: levels 0–6, thresholds 0 / 2 / 8 / 16 /
//! 26 / 38 / 52; `Threshold` is a `u16` at +0x10e, the heat at which the level *ends*).

use mw_assets::police::HeatLevel;

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
