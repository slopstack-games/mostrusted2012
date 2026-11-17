//! The UI sounds the game logic plays by field (`mw_logic::natives::Request::PlayUiSound`): the hooks
//! `OnOpenEasyDrive` and its siblings (0x6877b0 ...) read the UniqueId at
//! `[[DAT_010edb08]+4 +0xb27c] + [[...]+0xb278]+0x28 + field`, a field of the object behind the handle at +0xb278.
//! That handle is the `GlobalSound` root's (`SOUND/GLOBAL.BNDL` object 0x79e7, loaded by 0x4b8033) field at +0x14,
//! copied by 0x4b80fe next to +0x1c → +0xb270 (`VehicleSounds`, confirmed by `FUN_00594870`): `MiscSounds`, the
//! `Genesys.Gen.MiscSound` 0x5a953. Its `UI` struct (`cfac01f3`, at +0x40) holds eight UniqueIds at +0x4c..+0x68;
//! each names a UI sequence of wave items (1836401: one 3 s wave, 1836407: five waves 0.1 s apart).
//!
//! | field | hash | sequence (install) | played by |
//! |---|---|---|---|
//! | 0x4c | `c4acda7e` | 1836407 | `OnCloseEasyDriveMods` |
//! | 0x50 | `b6d0d5ee` | 1836401 | `OnOpenEasyDrive` |
//! | 0x54 | `34169795` | 1836405 | `OnOpenEasyDriveMods` |
//! | 0x58 | `619dd459` | 1836335 | `OnSelectEasyDriveOption` |
//! | 0x5c | `295705ad` | 1836058 | |
//! | 0x60 | `fd94fb0b` | 1836333 | `OnStartEvent`, `StopAllSequences` |
//! | 0x64 | `29510402` | 1836331 | `UpdatePendingRollout_EasyDrive_Stock` |
//! | 0x68 | `425b045b` | 1836060 | |

use mw_bundle::ResourceType;
use mw_formats::genesys::Value;

use crate::AssetStore;

/// The field hash of `MiscSound.UI` (a struct at +0x40).
const UI: u32 = 0xcfac_01f3;
/// The `UI` struct's UniqueId fields by offset in `MiscSound` (`genesys-types`: `MiscSound.UI` at +0x40, its
/// fields at +12..+40).
const FIELDS: [(u32, u32); 8] = [
    (0x4c, 0xc4ac_da7e),
    (0x50, 0xb6d0_d5ee),
    (0x54, 0x3416_9795),
    (0x58, 0x619d_d459),
    (0x5c, 0x2957_05ad),
    (0x60, 0xfd94_fb0b),
    (0x64, 0x2951_0402),
    (0x68, 0x425b_045b),
];

impl AssetStore {
    /// The UI sound sequence (UniqueId) at `field` of the global `MiscSound` (see the module doc); `None` when the
    /// field is not one of its `UI` ids or the id is 0 (the hooks then play nothing).
    pub fn ui_sound(&self, field: u32) -> Option<u32> {
        let hash = FIELDS.iter().find(|(f, _)| *f == field)?.1;
        let b = self.bundle("SOUND/GLOBAL.BNDL")?;
        let misc = b
            .entries_of_type(ResourceType::GenesysObject)
            .filter_map(|e| self.decode_genesys(&b.load(e).ok()?).ok())
            .find(|o| o.type_name == "Genesys.Gen.MiscSound")?;
        let Some(Value::Struct(ui)) = misc.get(UI) else { return None };
        match ui.get(hash)? {
            Value::UInt(id) if *id != 0 => u32::try_from(*id).ok(),
            Value::Int(id) if *id > 0 => u32::try_from(*id).ok(),
            _ => None,
        }
    }
}
