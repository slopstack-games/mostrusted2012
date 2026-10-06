//! Easydrive, the in-drive d-pad menu, as the screen data defines it (research log 2026-10-06, session 5).
//!
//! A driving screen (`UI/SCREENS2/*.BNDL`, `Screen2`) holds widget groups. One group carries the menu widget
//! itself (`Widget_ShowLayouts` "EasyDrive": the layouts, the `EasyDrive::*` user actions and the
//! `EasyDrive_ScreenScript`) and the item *types* (`Widget_EasyDriveTypeDefinition`: LINKITEM, HOOKITEM,
//! RPCITEM, PAINTITEM ... with their bound properties and selection callbacks). Every other group that holds
//! a `Widget_EasyDriveTierDefinition` is one tier (page) of the menu: its HSM transition name (`TO_BASE`,
//! `TO_JACK` ...), a title string, and entry sources (`Widget.EZTier.ConstantItems` of type LINKITEM, ...);
//! next to it sit the item lists the tier's adapters expose (`Widget_EasyDriveLinkItemList` "EZLinkItems",
//! `Widget_EasyDriveHookItemList` "EZHookExitItems" ...). A link item's token names the tier it opens.

use mw_formats as fmt;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// The free-roam driving screen with the full Easydrive tree (jack spots, recommended cars, paint, mods).
pub const EASYDRIVE_SCREEN: &str = "UI/SCREENS2/371621.BNDL";

mod field {
    pub const GROUP_CHILDREN: u32 = 0xc741_3006;
    pub const TIER_TRANSITION: u32 = 0x66b6_6508;
    pub const TIER_TITLE: u32 = 0xb310_f1f5;
    pub const TIER_SOURCES: u32 = 0x7823_ea7b;
    pub const SOURCE_CONDITION: u32 = 0x053e_93c0;
    pub const SOURCE_LENGTH: u32 = 0x2205_fba3;
    pub const LIST_ENTRIES: u32 = 0xb7a8_1572;
    pub const ITEM_TOKEN: u32 = 0xb50f_f8fe;
    pub const ITEM_DESCRIPTION: u32 = 0xe42b_9ac3;
    pub const ITEM_SPEECH: u32 = 0xf358_534f;
    pub const HOOK_ARGUMENT: u32 = 0xaabe_34d0;
    pub const HOOK_EXIT: u32 = 0x3ab7_e7fa;
    pub const TYPE_PROPERTIES: u32 = 0xf170_f474;
    pub const TYPE_CALLBACKS: u32 = 0xacf1_6aa6;
    pub const TYPE_CALLBACKS2: u32 = 0x84a4_a000;
    pub const CALLBACK_ARGUMENT: u32 = 0xaabe_34d0;
    pub const CALLBACK_ENABLED: u32 = 0x7814_43e5;
}

/// Where a tier's entries come from: a data-model path the game's adapters fill, of one item type.
#[derive(Debug, Clone)]
pub struct EntrySource {
    /// Binding that must be true for the source to show (`Mode.RetryLastEventEnabled`); empty = always.
    pub condition: String,
    pub path: String,
    pub item_type: String,
}

/// One menu entry from an item list.
#[derive(Debug, Clone)]
pub struct Item {
    /// Link items: the HSM transition (= target tier name). Hook items: the code hook (`HookToken`).
    pub token: String,
    /// Language string id of the label.
    pub name: u32,
    pub description: u32,
    pub speech: i64,
    /// Hook items: the HSM exit after the hook (`HsmToken`: `DISMISS`, `DONE`, `TO_EVENT`) and the hook's
    /// argument (`HookArgument`, often a binding).
    pub exit: String,
    pub argument: String,
}

/// An item list widget (`EZLinkItems`, `EZHookExitItems` ...).
#[derive(Debug, Clone)]
pub struct ItemList {
    pub widget: String,
    /// `Link` or `Hook` (from the GeneSys type name).
    pub kind: String,
    pub items: Vec<Item>,
}

/// One page of the menu with the item lists of its widget group.
#[derive(Debug, Clone)]
pub struct Tier {
    /// Widget group index in the screen.
    pub group: usize,
    /// HSM transition name that opens this tier (`TO_BASE` is the root).
    pub transition: String,
    /// Language string id of the title (0 = none).
    pub title: u32,
    pub sources: Vec<EntrySource>,
    /// Other non-tier widgets in the group (`JSMANUFACT`, `PAINTITEM`, `EZStateJack` ...).
    pub widgets: Vec<String>,
    pub lists: Vec<ItemList>,
}

/// A selection callback of an item type: what choosing an item of that type does.
#[derive(Debug, Clone)]
pub struct Callback {
    pub name: String,
    pub argument: String,
    pub enabled_property: String,
}

/// `Widget_EasyDriveTypeDefinition`: an item type.
#[derive(Debug, Clone)]
pub struct ItemType {
    pub name: String,
    /// (property name, data path).
    pub properties: Vec<(String, String)>,
    pub callbacks: Vec<Callback>,
}

#[derive(Debug, Clone, Default)]
pub struct EasyDrive {
    pub types: Vec<ItemType>,
    pub tiers: Vec<Tier>,
}

impl EasyDrive {
    /// The tier a link token opens.
    pub fn tier(&self, transition: &str) -> Option<&Tier> {
        self.tiers.iter().find(|t| t.transition == transition)
    }
}

fn s(o: &Object, h: u32) -> String {
    match o.get(h) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn n(o: &Object, h: u32) -> i64 {
    match o.get(h) {
        Some(Value::Int(v)) => *v,
        Some(Value::UInt(v)) => *v as i64,
        _ => 0,
    }
}

fn structs(o: &Object, h: u32) -> impl Iterator<Item = &Object> {
    let a: &[Value] = match o.get(h) {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    a.iter().filter_map(|v| if let Value::Struct(o) = v { Some(o) } else { None })
}

fn short(type_name: &str) -> &str {
    type_name.rsplit('.').next().unwrap_or(type_name)
}

/// Every widget group (its children) in a screen object, depth first.
fn groups(o: &Object, out: &mut Vec<Vec<Object>>) {
    for (h, v) in &o.fields {
        let mut walk = |v: &Value| {
            if let Value::Struct(c) = v {
                groups(c, out)
            }
        };
        match v {
            Value::Array(a) if *h == field::GROUP_CHILDREN => out
                .push(a.iter().filter_map(|v| if let Value::Struct(o) = v { Some(o.clone()) } else { None }).collect()),
            Value::Array(a) => a.iter().for_each(&mut walk),
            v => walk(v),
        }
    }
}

/// Builds the menu model from a decoded `Screen2` object.
pub fn from_screen(screen: &Object) -> EasyDrive {
    let mut all = Vec::new();
    groups(screen, &mut all);
    let mut ed = EasyDrive::default();
    for (gi, group) in all.iter().enumerate() {
        let mut tier: Option<Tier> = None;
        let (mut widgets, mut lists) = (Vec::new(), Vec::new());
        for w in group {
            let name = s(w, field_hash("Name"));
            match short(&w.type_name) {
                "Widget_EasyDriveTypeDefinition" => ed.types.push(ItemType {
                    name,
                    properties: structs(w, field::TYPE_PROPERTIES)
                        .map(|p| (s(p, field_hash("Name")), s(p, field_hash("Path"))))
                        .collect(),
                    callbacks: structs(w, field::TYPE_CALLBACKS)
                        .chain(structs(w, field::TYPE_CALLBACKS2))
                        .map(|c| Callback {
                            name: s(c, field_hash("Name")),
                            argument: s(c, field::CALLBACK_ARGUMENT),
                            enabled_property: s(c, field::CALLBACK_ENABLED),
                        })
                        .collect(),
                }),
                "Widget_EasyDriveTierDefinition" if tier.is_none() => {
                    tier = Some(Tier {
                        group: gi,
                        transition: s(w, field::TIER_TRANSITION),
                        title: n(w, field::TIER_TITLE) as u32,
                        sources: structs(w, field::TIER_SOURCES)
                            .map(|o| EntrySource {
                                condition: s(o, field::SOURCE_CONDITION),
                                path: s(o, field_hash("Path")),
                                item_type: s(o, field_hash("Type")),
                            })
                            .collect(),
                        widgets: Vec::new(),
                        lists: Vec::new(),
                    })
                }
                t @ ("Widget_EasyDriveLinkItemList" | "Widget_EasyDriveHookItemList") => lists.push(ItemList {
                    widget: name,
                    kind: if t.contains("Link") { "Link" } else { "Hook" }.into(),
                    items: structs(w, field::LIST_ENTRIES)
                        .map(|i| Item {
                            token: s(i, field::ITEM_TOKEN),
                            name: n(i, field_hash("Name")) as u32,
                            description: n(i, field::ITEM_DESCRIPTION) as u32,
                            speech: n(i, field::ITEM_SPEECH),
                            exit: s(i, field::HOOK_EXIT),
                            argument: s(i, field::HOOK_ARGUMENT),
                        })
                        .collect(),
                }),
                _ => widgets.push(name),
            }
        }
        if let Some(mut t) = tier {
            t.widgets = widgets;
            t.lists = lists;
            ed.tiers.push(t);
        }
    }
    ed
}

impl AssetStore {
    /// The Easydrive menu model of a driving screen bundle (e.g. [`EASYDRIVE_SCREEN`]).
    pub fn easydrive(&self, bundle: &str) -> Result<EasyDrive> {
        let b = self.bundle(bundle).ok_or(crate::Error::Missing(mw_bundle::ResourceId::NULL))?;
        for e in b.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
            let res = b.load(e)?;
            let Some(ty) = res.import_at(0) else { continue };
            if self.genesys_type(ty).is_some_and(|t| t.name == "Genesys.Gen.Screen2") {
                return Ok(from_screen(&self.decode_genesys(&res)?));
            }
        }
        Err(crate::Error::Missing(mw_bundle::ResourceId::NULL))
    }
}

/// Label of a language id, or the id itself.
pub fn label(lang: &fmt::Language, id: u32) -> String {
    lang.get(id).map(str::to_owned).unwrap_or_else(|| format!("#{id}"))
}
