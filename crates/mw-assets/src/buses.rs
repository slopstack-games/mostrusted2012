//! The game's bus mix (`SOUND/MIXER_CHANNELS.BNDL`): which bus a mixing group plays into and the gain of the
//! bus chain from there to the master, as a level relative to the player's engine.
//!
//! `Genesys.Gen.BusRoute { AssetName, Mixing_Groups, Main_Send (the next route's UniqueId, 0 = the output),
//! Compressor_Send }` names 23 routes ("Player Car", "Player Sub", "Player Collisions", "World", "Dominant World",
//! "Competitor", "Master World", "Final", "Master", ...); `Genesys.Gen.BusMixerChannel { Routing (its route), EQ
//! curve, Compressor, nine gains }` is each route's bus. The nine gains (`3ca61a54`, `79a2cbf6`, `8b61e126`,
//! `eecf2aca`, `f31e8e7e`, `0e4f2179`, `bec6fe4f`, `fca6257d`, `d7f83c28`) are one per mix state; the state the
//! game uses in free roam was not traced. [`STATE`] (`8b61e126`) is the first one with every bus audible (the
//! first mutes the car and the master, the second the music, the fifth and later the speech): a model decision.
//! Over the routes the sound effects use, the states differ by at most 0.3 dB (the "Player Car" bus: 0.745..0.794).
//!
//! The player's engine bank is normalised to a common level (`EngineBank::normalise`, a model), so the bus gains
//! are applied relative to the engine's route: [`BusMix::relative`] = the chain gain of a group's route over the
//! chain gain of "Player Car" (the route of the engine banks' channels, mixing group 29310). Not ported: the
//! buses' EQ curves and compressors (incl. the side chains named by `Compressor_Send`).

use std::collections::HashMap;

use mw_bundle::ResourceType;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::sound::number;

/// The bus gain field read (one of the nine mix states; see the module docs).
pub const STATE: u32 = 0x8b61_e126;
/// The engine banks' route: "Player Car" (mixing group 29310).
pub const ENGINE_GROUP: u32 = 29310;

/// Chain gains per mixing group.
#[derive(Debug, Clone, Default)]
pub struct BusMix {
    /// Mixing group → (route name, chain gain to the output).
    groups: HashMap<u32, (String, f32)>,
}

fn handle_of(v: Option<&Value>) -> Option<mw_bundle::ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

impl BusMix {
    /// `MW_AUDIO_NO_BUS=1` leaves the bus gains out (every group 1, as before they were ported: A/B).
    pub fn load(store: &AssetStore) -> Self {
        if std::env::var_os("MW_AUDIO_NO_BUS").is_some() {
            return Self::default();
        }
        let Some(b) = store.bundle("SOUND/MIXER_CHANNELS.BNDL") else { return Self::default() };
        let objects: Vec<(mw_bundle::ResourceId, Object)> = b
            .entries_of_type(ResourceType::GenesysObject)
            .filter_map(|e| Some((e.id, store.decode_genesys(&b.load(e).ok()?).ok()?)))
            .collect();
        let uid = |o: &Object| number(o.get(0x655d_3f5f)).map(|v| v as u32);
        // Routes by UniqueId: (resource id, name, next route, groups).
        let mut routes: HashMap<u32, (mw_bundle::ResourceId, String, u32, Vec<u32>)> = HashMap::new();
        for (id, o) in objects.iter().filter(|(_, o)| o.type_name == "Genesys.Gen.BusRoute") {
            let Some(u) = uid(o) else { continue };
            let name = match o.get(field_hash("AssetName")) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            };
            let next = number(o.get(field_hash("Main_Send"))).unwrap_or(0.0) as u32;
            let groups = match o.get(field_hash("Mixing_Groups")) {
                Some(Value::Array(a)) => a.iter().filter_map(|v| number(Some(v))).map(|g| g as u32).collect(),
                _ => Vec::new(),
            };
            routes.insert(u, (*id, name, next, groups));
        }
        // Each route's own bus gain.
        let mut gain: HashMap<mw_bundle::ResourceId, f32> = HashMap::new();
        for (_, o) in objects.iter().filter(|(_, o)| o.type_name == "Genesys.Gen.BusMixerChannel") {
            if let Some(r) = handle_of(o.get(field_hash("Routing"))) {
                gain.insert(r, number(o.get(STATE)).unwrap_or(1.0));
            }
        }
        let chain = |mut u: u32| {
            let mut g = 1.0f32;
            for _ in 0..32 {
                let Some((id, _, next, _)) = routes.get(&u) else { break };
                g *= gain.get(id).copied().unwrap_or(1.0);
                if *next == 0 {
                    break;
                }
                u = *next;
            }
            g
        };
        let mut groups = HashMap::new();
        for (u, (_, name, _, gs)) in &routes {
            for &g in gs.iter().filter(|&&g| g != 0) {
                groups.insert(g, (name.clone(), chain(*u)));
            }
        }
        Self { groups }
    }

    /// The route a mixing group plays into and its chain gain.
    pub fn route(&self, group: u32) -> Option<(&str, f32)> {
        self.groups.get(&group).map(|(n, g)| (n.as_str(), *g))
    }

    /// A group's level against the engine's ("Player Car"): 1 for groups that are no route's (channels that play
    /// into another channel of their bank).
    pub fn relative(&self, group: u32) -> f32 {
        match (self.groups.get(&group), self.groups.get(&ENGINE_GROUP)) {
            (Some((_, g)), Some((_, e))) if *e > 0.0 => g / e,
            _ => 1.0,
        }
    }
}
