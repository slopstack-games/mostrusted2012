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
    /// `NitrousParameters` id → its burn per second.
    pub nitrous_burns: Vec<(String, f32)>,
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
    /// The stock handling with the selection applied, and the selected nitrous variant's burn per second.
    pub fn modded(&self, sel: &ModSelection) -> (VehicleHandling, Option<f32>) {
        let mut h = self.stock.clone();
        let mut mods: Vec<Modification> = Vec::new();
        let mut burn = None;
        for &(slot, cat) in &sel.0 {
            let Some(c) = self.mods.get(cat).and_then(|s| s.get(slot)) else { continue };
            mods.extend(&c.modifications);
            if let Some(id) = &c.nitrous {
                burn = self.nitrous_burns.iter().find(|(i, _)| i == id).map(|b| b.1).or(burn);
            }
        }
        mw_assets::mods::apply_to(&mut h, &mods);
        (h, burn)
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
pub fn apply_live(sel: Res<ModSelection>, cat: Res<ModCatalogue>, mut cars: Query<&mut crate::drive::Car>) {
    if !sel.is_changed() || sel.is_added() {
        return;
    }
    let (h, burn) = cat.modded(&sel);
    for mut car in &mut cars {
        car.params = crate::drive::CarParams::new(&h);
        let mut n = mw_handling::nitrous::NitrousParams::new(&h);
        n.burn = burn.unwrap_or(n.burn);
        car.nitrous_params = n;
    }
    info!("mods: {}", cat.describe(&sel));
}
