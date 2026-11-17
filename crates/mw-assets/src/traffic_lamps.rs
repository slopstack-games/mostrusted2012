//! Where the junctions' traffic lights stand (docs/decomp.md "Traffic lights: the lamps").
//!
//! A junction light group names its lamps twice: a `u16` light id (the index of the lamp's 8-byte record in the
//! traffic system, world +0x65e20) and a `u32` object handle (`TrafficLightGroup::light_objects`) that
//! `FUN_005ef950` copies into that record. The handle is the placed object's instance id: 101 of the 104 handles
//! equal the +0x4c word of a `CompoundInstanceList` record (`mwtool traffic`), all of them traffic-light compounds
//! (`aa60bafa` ×62, `241bbd82` ×24, `e86efde1` ×15). The light cycle's messages 0xb / 0xc carry that handle;
//! `FUN_005539b0` looks it up in the world's id map.

use std::collections::HashMap;

use mw_bundle::ResourceType;
use mw_formats as fmt;

use crate::AssetStore;

impl AssetStore {
    /// The world position of each placed compound or dynamic object whose instance id is in `handles`.
    pub fn instance_positions(&self, handles: &[u32]) -> HashMap<u32, [f32; 3]> {
        let mut out = HashMap::new();
        for path in self.bundle_paths().to_vec() {
            if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")) {
                continue;
            }
            // Opened outside the store's cache: every track unit, once.
            let Ok(bundle) =
                self.game.read(&path).map_err(mw_bundle::Error::from).and_then(mw_bundle::Bundle::from_bytes)
            else {
                continue;
            };
            for ty in [ResourceType::CompoundInstanceList, ResourceType::DynamicInstanceList] {
                for e in bundle.entries_of_type(ty) {
                    let Ok(res) = bundle.load(e) else { continue };
                    let Ok(list) = fmt::ObjectInstanceList::parse(&res) else { continue };
                    for i in list.instances {
                        for id in i.ids {
                            if id != 0 && handles.contains(&id) {
                                let [x, y, z, _] = i.transform[3];
                                out.insert(id, [x, y, z]);
                            }
                        }
                    }
                }
            }
        }
        out
    }
}
