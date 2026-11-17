//! Performance mods on the driven car (docs/decomp.md "Nitrous and upgrades"): a selection of one category per
//! slot of the car's `PerformanceUpgrades`, routed through `mw_assets::mods::apply_to` (the game's
//! `FUN_00574870`) whenever the car spawns or the selection changes. Every mod is available: the game's unlocks
//! (won events) are not modelled.

use bevy::prelude::*;
use mw_assets::handling::VehicleHandling;
use mw_assets::mods::{ModChoice, Modification};

/// The chosen mods: (slot index, category index) as `mwtool mods` prints them; at most one per slot.
#[derive(Resource, Default, Clone, Debug)]
pub struct ModSelection(pub Vec<(usize, usize)>);

/// The driven car's mod data and stock handling, filled when it spawns (`drive.rs`).
#[derive(Resource, Default, Clone, Debug)]
pub struct ModCatalogue {
    pub stock: VehicleHandling,
    /// Per category, the slots (`mw_assets::mods::performance_upgrades`).
    pub mods: Vec<Vec<ModChoice>>,
    /// `NitrousParameters` id → its rules (every nitrous mod's variant).
    pub nitrous: Vec<(String, mw_handling::nitrous::NitrousRules)>,
    /// The car's nitrous without a nitrous mod (`Vehicle_Gameplay.Nitrous_Online`, handling.md "Model
    /// decisions": the original's offline default `Nitrous_Offline` has the nitrous disabled).
    pub default_nitrous: mw_handling::nitrous::NitrousRules,
    /// Language-resolved slot names, per category and slot.
    pub names: Vec<Vec<String>>,
}

impl ModSelection {
    /// `--mods slot=category[,…]`: the slot as its index or hex hash, the category as its index.
    pub fn parse(s: &str, mods: &[Vec<ModChoice>]) -> Self {
        let mut out = Vec::new();
        for item in s.split(',').filter(|i| !i.is_empty()) {
            let Some((slot, cat)) = item.split_once('=') else { continue };
            let cat: usize = cat.trim().parse().unwrap_or(0);
            let slots = mods.get(cat).map(Vec::as_slice).unwrap_or_default();
            let slot = slot.trim().parse::<usize>().ok().or_else(|| {
                let h = u32::from_str_radix(slot.trim(), 16).ok()?;
                slots.iter().position(|c| c.slot == h)
            });
            // Without the catalogue (command line, before the car loads) indices are taken as given.
            if let Some(slot) = slot.filter(|&s| slots.is_empty() || s < slots.len()) {
                out.retain(|&(s, _)| s != slot);
                out.push((slot, cat));
            }
        }
        Self(out)
    }
}

impl ModCatalogue {
    /// The stock handling with the selection applied, and the nitrous rules: the selected nitrous mod's
    /// `NitrousParameters` (`FUN_0065de50`), else the default.
    pub fn modded(&self, sel: &ModSelection) -> (VehicleHandling, mw_handling::nitrous::NitrousRules) {
        let mut h = self.stock.clone();
        let mut mods: Vec<Modification> = Vec::new();
        let mut rules = self.default_nitrous;
        for &(slot, cat) in &sel.0 {
            // Game slots 11 and 13 need the Terminal Velocity pack (`FUN_00678e60`, `dlc.rs`).
            if crate::dlc::mod_field_locked(slot) {
                continue;
            }
            let Some(c) = self.mods.get(cat).and_then(|s| s.get(slot)) else { continue };
            mods.extend(&c.modifications);
            if let Some(id) = &c.nitrous {
                rules = self.nitrous.iter().find(|(i, _)| i == id).map_or(rules, |b| b.1);
            }
        }
        mw_assets::mods::apply_to(&mut h, &mods);
        h.tyre_class = self.tyre_class(sel);
        (h, rules)
    }

    /// The tyre class a selection fits (`None` = stock): the `Tyres` of a fitted OFF-ROAD (`0d43dacc`, game slot 0,
    /// `FUN_00589a60`) or TRACK (`dd869f8b`, game slot 1, `FUN_00589b00`) `TyreUpgrade` of the chosen level. The
    /// third tyre slot (REINFLATES, `5d0e0d89`) is not read by either, so it leaves the class alone.
    pub fn tyre_class(&self, sel: &ModSelection) -> Option<u8> {
        sel.0.iter().find_map(|&(slot, cat)| {
            let c = self.mods.get(cat)?.get(slot)?;
            if !TYRE_CLASS_SLOTS.contains(&c.slot) {
                return None;
            }
            c.tyre.and_then(|t| u8::try_from(t).ok())
        })
    }

    /// "slot:category name" of each selected mod, for test output.
    pub fn describe(&self, sel: &ModSelection) -> String {
        let v: Vec<String> = sel
            .0
            .iter()
            .filter_map(|&(slot, cat)| {
                let c = self.mods.get(cat)?.get(slot)?;
                let name = self.names.get(cat).and_then(|n| n.get(slot)).cloned().unwrap_or_default();
                Some(format!("{slot}={cat} {:08x} {} \"{name}\"", c.slot, c.kind))
            })
            .collect();
        if v.is_empty() { "none".into() } else { v.join("; ") }
    }
}

/// Re-apply the selection to the driven car as soon as it changes (the game applies mods while driving).
pub fn apply_live(
    sel: Res<ModSelection>,
    cat: Res<ModCatalogue>,
    mut cars: Query<&mut crate::drive::Car, With<crate::drive::Player>>,
) {
    if !sel.is_changed() || sel.is_added() {
        return;
    }
    let (h, rules) = cat.modded(&sel);
    for mut car in &mut cars {
        car.params = crate::drive::CarParams::new(&h);
        let mut n = mw_handling::nitrous::NitrousParams::new(&h);
        n.rules = rules;
        car.nitrous_params = n;
    }
    info!("mods: {}", cat.describe(&sel));
}

/// The mod classes of Easydrive's "Customize car" branch and the slots of each, by the slot fields' hashes. The
/// field names (cracked with `mwtool genesys-crack`: `Body1`/`2`, `Chassis1`–`3`, `Nitrous1`–`4`, `PowerTrain1`/`2`
/// in `PerformanceUpgrades.Category`, next to `StockBody` … `StockTyres` in `PerformanceUpgrades`) give the
/// classes; the tyre class is the three `TyreUpgrade` slots plus DRIFT TIRES (`d29bfc4c`). The class labels are
/// ours: the game's strings for them were not found in the data.
pub const CLASSES: [(&str, &[u32]); 5] = [
    ("TIRES", &[0xdd86_9f8b, 0x0d43_dacc, 0x5d0e_0d89, 0xd29b_fc4c]),
    ("NITROUS", &[0x201d_82f0, 0x201d_82f3, 0x201d_82f2, 0x201d_82f5]),
    ("CHASSIS", &[0xe706_061e, 0xe706_061d, 0xe706_061c]),
    ("BODY", &[0xe397_7263, 0xe397_7260]),
    ("TRANSMISSION", &[0x05cb_72b0, 0x05cb_72b3]),
];

/// The unnamed `ModifierUpgrade` slot that goes with a tyre slot: it sorts directly before it among the fields
/// (fields are stored sorted by name), so it shares the tyre's name prefix (inferred; its own name is not cracked).
const TYRE_MODIFIERS: [(u32, u32); 2] = [(0x0d43_dacc, 0x6ffa_60e4), (0xdd86_9f8b, 0x1456_09b0)];

/// The `TyreUpgrade` slots whose `Tyres` becomes the car's tyre class: OFF-ROAD and TRACK.
const TYRE_CLASS_SLOTS: [u32; 2] = [0x0d43_dacc, 0xdd86_9f8b];

/// Category index of the base level; 0 is the PRO level (decomp.md "Nitrous and upgrades").
pub const BASE: usize = 1;
pub const PRO: usize = 0;

impl ModCatalogue {
    fn slot_index(&self, hash: u32) -> Option<usize> {
        self.mods.get(BASE)?.iter().position(|c| c.slot == hash)
    }

    pub fn name(&self, slot: usize, cat: usize) -> String {
        self.names.get(cat).and_then(|n| n.get(slot)).cloned().unwrap_or_default()
    }

    /// The classes this car has mods for: (class index, label).
    pub fn classes(&self) -> Vec<(usize, &'static str)> {
        CLASSES.iter().enumerate().filter(|(k, _)| !self.class_slots(*k).is_empty()).map(|(k, c)| (k, c.0)).collect()
    }

    /// The selectable slots of a class. A slot whose two levels carry the same name and data (the 911's third
    /// nitrous slot) is the class's default and is left to "stock".
    pub fn class_slots(&self, class: usize) -> Vec<usize> {
        CLASSES[class]
            .1
            .iter()
            .filter_map(|&h| self.slot_index(h))
            .filter(|&s| self.name(s, BASE) != self.name(s, PRO) && !self.name(s, BASE).is_empty())
            .collect()
    }

    fn class_all_slots(&self, class: usize) -> Vec<usize> {
        let mut v: Vec<usize> = CLASSES[class].1.iter().filter_map(|&h| self.slot_index(h)).collect();
        for (tyre, modifier) in TYRE_MODIFIERS {
            if CLASSES[class].1.contains(&tyre) {
                v.extend(self.slot_index(modifier));
            }
        }
        v
    }

    /// The class's equipped (slot, category), if any.
    pub fn equipped(&self, sel: &ModSelection, class: usize) -> Option<(usize, usize)> {
        let slots = self.class_slots(class);
        sel.0.iter().copied().find(|(s, _)| slots.contains(s))
    }

    /// Equip one mod of a class (replacing the class's current one), or return the class to stock with `None`.
    pub fn equip(&self, sel: &mut ModSelection, class: usize, choice: Option<(usize, usize)>) {
        let all = self.class_all_slots(class);
        sel.0.retain(|(s, _)| !all.contains(s));
        if let Some((slot, cat)) = choice {
            sel.0.push((slot, cat));
            let hash = self.mods.get(BASE).and_then(|m| m.get(slot)).map(|c| c.slot);
            if let Some((_, modifier)) = TYRE_MODIFIERS.iter().find(|(t, _)| Some(*t) == hash)
                && let Some(m) = self.slot_index(*modifier)
            {
                sel.0.push((m, cat));
            }
        }
    }
}
