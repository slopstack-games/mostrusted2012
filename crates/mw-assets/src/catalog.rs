//! Human-facing catalogue data: localised strings and the list of cars.

use mw_bundle::ResourceType;
use mw_formats as fmt;

use crate::{AssetStore, Error, Result};

/// Default language bundle (index 1 is English).
pub const DEFAULT_LANGUAGE: &str = "UI/LANGUAGE/0001.BNDL";

/// A car the game knows about, joined with its name and bundles.
#[derive(Debug, Clone)]
pub struct VehicleInfo {
    pub id: u32,
    pub name: String,
    pub year: u16,
    /// The vehicle list's `GamePackID` (+0x2c): 17127 = the base game, else a DLC pack.
    pub game_pack: u32,
    /// Game-relative paths of the high/low detail bundles, if the car has its own.
    pub hi_bundle: Option<String>,
    pub lo_bundle: Option<String>,
}

impl AssetStore {
    /// Load a language table, e.g. [`DEFAULT_LANGUAGE`].
    pub fn language(&self, bundle: &str) -> Result<fmt::Language> {
        let b = self.bundle(bundle).ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        let e = b.entries_of_type(ResourceType::Language).next().ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        Ok(fmt::Language::parse(&b.load(e)?)?)
    }

    /// Every entry of the vehicle list with names resolved through `lang`.
    pub fn vehicles(&self, lang: &fmt::Language) -> Result<Vec<VehicleInfo>> {
        let b = self.bundle("VEHICLES/VEHICLELIST.BNDL").ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        let e =
            b.entries_of_type(ResourceType::VehicleList).next().ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        let list = fmt::VehicleList::parse(&b.load(e)?)?;
        let bundle = |id: u32, lod: &str| {
            let p = format!("VEHICLES/VEH_{id}_{lod}.BNDL");
            self.bundle_paths().iter().any(|b| b.to_string_lossy().eq_ignore_ascii_case(&p)).then_some(p)
        };
        Ok(list
            .entries
            .iter()
            .map(|v| VehicleInfo {
                id: v.id,
                name: lang.get(v.name_string).unwrap_or("?").to_string(),
                year: v.year,
                game_pack: v.game_pack,
                hi_bundle: bundle(v.id, "HI"),
                lo_bundle: bundle(v.id, "LO"),
            })
            .collect())
    }

    /// The vehicle list's entry for a car id (`VEHICLES/VEHICLELIST.BNDL`).
    pub fn vehicle_entry(&self, id: u32) -> Result<Option<fmt::VehicleEntry>> {
        let b = self.bundle("VEHICLES/VEHICLELIST.BNDL").ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        let e =
            b.entries_of_type(ResourceType::VehicleList).next().ok_or(Error::Missing(mw_bundle::ResourceId::NULL))?;
        Ok(fmt::VehicleList::parse(&b.load(e)?)?.entries.into_iter().find(|v| v.id == id))
    }

    /// Find a car by (case-insensitive) name substring or numeric ID.
    pub fn find_vehicle(&self, query: &str) -> Result<Option<VehicleInfo>> {
        let lang = self.language(DEFAULT_LANGUAGE)?;
        let q = query.to_ascii_lowercase();
        let mut all = self.vehicles(&lang)?;
        all.retain(|v| v.hi_bundle.is_some());
        Ok(all
            .iter()
            .find(|v| v.id.to_string() == q || v.name.to_ascii_lowercase() == q)
            .or_else(|| all.iter().find(|v| v.name.to_ascii_lowercase().contains(&q)))
            .cloned())
    }
}
