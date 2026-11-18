//! UI data: screens, layouts, text styles, fonts (see `mw_formats::ui` for the format).

use std::collections::HashMap;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;
use mw_formats::ui::{Element, ElementKind, Layout, LayoutInstance, Screen, TextStyle};

use crate::{AssetStore, Error, Result};

/// The free-roam driving HUD screen, `Screen2` "FreedriveHud" (the same bundle as Easydrive's menu): the
/// speed-point counter, minimap, nitrous gauge, the Easydrive prompt and its backing (`EngineOffHud`). Until
/// 2026-10-19 the HUD read 371670, "AmbushHud" (the Ambush event's screen: same widgets but no `EngineOffHud`
/// backing, `Speedwall` / `Recommends` instead of `HackEasyDrive`).
pub const FREE_ROAM_HUD: &str = "UI/SCREENS2/371621.BNDL";

/// Ink of a key's name inside a key box and of the box (the prompt labels' dark and light, linear).
const KEY_INK: [f32; 3] = [0.005_181_5, 0.005_181_5, 0.005_181_5];
const KEY_BOX: [f32; 3] = [0.830_77, 0.830_77, 0.830_77];

/// The `WorldPaintMap` that paints the districts over the world (`GAMELOGIC/GAMEPLAY.BNDL`): its entries are the
/// `PaintLayer`s of `GameLogicConfig.DistrictConfig.Districts` (10 districts, 10 entries besides "nothing").
pub const DISTRICT_PAINT_MAP: ResourceId = ResourceId(0x0100_0000_0002_823e);
/// The `GameLogicConfig` object (`GLOBALCONFIG.BNDL`, its only one).
const GAME_LOGIC_CONFIG: ResourceId = ResourceId(0x0100_0000_0004_5e6f);

/// The districts (`Players.LocalPlayer.CurrentDistrictName`, the plate above the minimap): the paint map and,
/// per paint layer, the district's name (Language id). The binding's code (`Player.CurrentDistrictName`, a
/// field at player +0x300 registered in the `Player` binding table) was not traced to its writer; the link
/// from a position to a name through this map is our reading of the data (each paint layer is exactly one
/// district's, and the plate at the original's start reads FAIRHAVEN, the district painted there).
#[derive(Debug, Clone)]
pub struct Districts {
    pub map: fmt::WorldPaintMap,
    /// Paint layer (UniqueId) → district name (Language id).
    pub names: HashMap<u32, u32>,
}

impl Districts {
    /// The name (Language id) of the district painted at a world position.
    pub fn name_at(&self, x: f32, z: f32) -> Option<u32> {
        self.map.entry_at(x, z).and_then(|layer| self.names.get(&layer).copied())
    }
}

impl AssetStore {
    /// [`Districts`]: `GameLogicConfig` field `DistrictConfig.Districts[]` {`Name`,
    /// `PaintLayer`} and [`DISTRICT_PAINT_MAP`].
    pub fn districts(&self) -> Result<Districts> {
        use fmt::genesys::{Value, field_hash as h};
        let cfg = self.genesys_object(GAME_LOGIC_CONFIG)?;
        let num = |o: &fmt::genesys::Object, f: &str| match o.get(h(f)) {
            Some(Value::UInt(u)) => Some(*u as u32),
            Some(Value::Int(i)) => Some(*i as u32),
            _ => None,
        };
        let mut names = HashMap::new();
        if let Some(Value::Struct(dc)) = cfg.get(h("DistrictConfig"))
            && let Some(Value::Array(list)) = dc.get(h("Districts"))
        {
            for d in list {
                if let Value::Struct(d) = d
                    && let (Some(name), Some(layer)) = (num(d, "Name"), num(d, "PaintLayer"))
                {
                    names.insert(layer, name);
                }
            }
        }
        let map = fmt::WorldPaintMap::parse(&self.load(DISTRICT_PAINT_MAP)?)?;
        Ok(Districts { map, names })
    }
}

/// A tab of a screen's `Widget_NavigationTabs` (the menus' bottom bar: MWL, MAP, MPLAYER, FRIENDS, SETTINGS,
/// STORE): the cell layouts bind `Widget.Tabs.TabList[#].Image` / `.Name` to these.
#[derive(Debug, Clone)]
pub struct NavTab {
    /// `TabName` (the page's token).
    pub token: String,
    /// `Image`: the icon texture.
    pub image: Option<ResourceId>,
    /// `Name`: Language id.
    pub name: u32,
}

impl AssetStore {
    /// The tabs of the first `Widget_NavigationTabs` of a screen: its `Tabs` list (six on the map screen 899991;
    /// a second list, field `d730cb94`, holds five of them and is not used here).
    pub fn navigation_tabs(&self, screen: &str) -> Result<Vec<NavTab>> {
        use fmt::genesys::{Value, field_hash as h};
        let o = self.ui_screen_object(screen)?;
        let mut widgets = Vec::new();
        crate::frontend::collect(&o, "Genesys.Gen.Widget_NavigationTabs", &mut widgets);
        let w = widgets.first().ok_or(Error::Missing(ResourceId::NULL))?;
        let Some(Value::Array(tabs)) = w.get(h("Tabs")) else { return Err(Error::Missing(ResourceId::NULL)) };
        Ok(tabs
            .iter()
            .filter_map(|t| match t {
                Value::Struct(t) => Some(NavTab {
                    token: match t.get(h("TabName")) {
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    image: match t.get(h("Image")) {
                        Some(Value::Handle(id)) => *id,
                        _ => None,
                    },
                    name: match t.get(h("Name")) {
                        Some(Value::UInt(u)) => *u as u32,
                        Some(Value::Int(i)) => *i as u32,
                        _ => 0,
                    },
                }),
                _ => None,
            })
            .collect())
    }
}

/// A point-of-interest look ([`fmt::ui::PoiVisuals`]) with its layouts.
#[derive(Debug, Clone)]
pub struct PoiVisual {
    pub id: u32,
    /// What game code names it by (`CheckPointsConfig.NormalPoi`, `MapPoiConfig` fields).
    pub key: u32,
    /// The icon drawn on the map (its instance translates to the `Pos` binding).
    pub map: Option<PlacedLayout>,
    /// Layouts drawn in the world at the point's transform (`Pos`, `Rot`, `DistanceToCamera` bindings).
    pub world: Vec<PlacedLayout>,
    /// The world layouts face the camera ([`fmt::ui::PoiVisuals::face_camera`]).
    pub face_camera: bool,
}

/// Point-of-interest keys (see [`AssetStore::poi_keys`]). Offsets are into the config structs as
/// `FUN_007545b0` reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoiKeys {
    /// `CheckPointsConfig.NormalPoi` (+0x28, 1013519): the gate being driven to.
    pub gate: u32,
    /// `CheckPointsConfig` +0x24 (`e9d221fe`, 1013518): the gate after it.
    pub gate_preview: u32,
    /// `MapPoiConfig` +0x28 (`f4d5177c`, 1013510): the finish as the target.
    pub finish: u32,
    /// `MapPoiConfig` +0x2c (`8d67562d`, 1695650): the finish as the gate after the target.
    pub finish_preview: u32,
    /// `MapPoiConfig` `29401278` (1013507): its map icon is the player-coloured car arrow (which field the
    /// code reads for the local player was not traced).
    pub player: u32,
    /// `MapPoiConfig` `5bfb3553` (1013521): racer / cop arrows (not traced which field races use). The look
    /// itself switches between the racer and the cop arrow on `Players.PlayerBase[#].IsCop`.
    pub opponent: u32,
    /// `CollectablesConfig` +0x10 (`d71dba55`, 1444895): a body shop (`FUN_006ffba0`, kind 6).
    pub body_shop: u32,
    /// `CollectablesConfig` +0x48 (`507e9d87`, 1013508): a jack spot location (`FUN_006ffba0`, kinds '<', 'B',
    /// 'E', 'K', 'N', base game); its map icon is visible on `JackSpots.IsSwapSpotFound[#]`.
    pub jack_spot: u32,
    /// `CollectablesConfig` +0x34 (`a1b2d86a`, 955694): the jack spot look when `FUN_006fcf90` holds (the
    /// spot's car in a list of the profile; its icon has no visibility binding).
    pub jack_spot_owned: u32,
    /// `MapPoiConfig` +0x44 (`5626fb85`) and +0x40 (`5e37f18a`): the online meet-up's two looks (`SHOWMEETUPPO`,
    /// 0x751b10).
    pub meetup: [u32; 2],
}

/// The world map's item kinds (`UI.MapItemType.ItemType`), as the map code adds them (`FUN_008181b0`,
/// `FUN_008154f0`, `FUN_00817500`).
pub mod map_item {
    pub const LOCAL_PLAYER: u16 = 0;
    /// An offline event (`MapItemTypes[1]`).
    pub const EVENT: u16 = 1;
    pub const SPEED_CAMERA: u16 = 3;
    /// Billboards (`DataSource` "Jump").
    pub const BILLBOARD: u16 = 4;
    pub const SECURITY_GATE: u16 = 5;
    /// A jack spot location while `FUN_006fcf90` does not hold.
    pub const JACK_SPOT: u16 = 7;
    /// An offline event with `c8f26580` (+0xe9, first place only) set: the Most Wanted races.
    pub const MOST_WANTED_EVENT: u16 = 9;
    pub const BODY_SHOP: u16 = 13;
    /// A jack spot location while `FUN_006fcf90` holds.
    pub const JACK_SPOT_OWNED: u16 = 14;
}

/// The world map's filters (`Widget_MapFilters.Filter.FilterType`) and what `FUN_008181b0` lists for each.
pub mod map_filter {
    /// Events (`FUN_008154f0` with the driven car's rules).
    pub const RACES: u16 = 1;
    /// Trigger kinds '4' and 'G' as item type 3.
    pub const SPEED_CAMERAS: u16 = 3;
    /// Trigger kinds '2', 'A', 'D', 'J', 'M' as item type 4.
    pub const BILLBOARDS: u16 = 4;
    /// Trigger kind '0' as item type 5.
    pub const SECURITY_GATES: u16 = 5;
    /// Trigger kind '<' as item type 7 / 14 and the packs' kinds 'B', 'E', 'K', 'N' as 15–18.
    pub const JACK_SPOTS: u16 = 7;
    /// Autolog recommendations (events by other rules).
    pub const RECOMMENDS: u16 = 9;
    /// Body shops (trigger kind 6) as item type 13.
    pub const BODY_SHOPS: u16 = 12;
}

/// One `UI.MapItemType` of the world map's `LiveMapManager_ScreenScript` (1135185).
#[derive(Debug, Clone, PartialEq)]
pub struct MapItemType {
    /// `ItemType` (+0x28): the index the code files the entry under (`FUN_0080ec90`: `+0x2298 + 4 × type`).
    pub item_type: u16,
    /// `DataSource`: the option menu (a `Widget_MenuData` of the map screen) for the item.
    pub data_source: String,
    /// `650f3bfb` (+0x20): the point-of-interest key of the item's icon.
    pub icon: u32,
    /// `c3f39ca3` (+0x24): the key of the card shown for the selected item.
    pub card: u32,
    pub options_height: u16,
}

/// The world map's configuration (`LiveMapManager_ScreenScript` 1135185), as `FUN_0080ec90` reads it by
/// field name.
#[derive(Debug, Clone, Default)]
pub struct MapManagerConfig {
    pub item_types: Vec<MapItemType>,
    /// `CollectablesName`: (item type, language id) of the collectables' names ("SPEED CAMERA", ...).
    pub collectable_names: Vec<(u16, u32)>,
    /// `SelectionPOI` (1135303): the cursor.
    pub selection_poi: u32,
    /// `AIPlayerPOI` / `AICopPOI` (both 1135615) and `BlackListDriver` (1444899): racers, cops, Most Wanted
    /// racers (`FUN_00808290`).
    pub ai_player_poi: u32,
    pub ai_cop_poi: u32,
    pub blacklist_driver_poi: u32,
    /// `CheckpointPOI` (1135616) / `FinishLinePOI` (1135617): the race route's gates on the map.
    pub checkpoint_poi: u32,
    pub finish_line_poi: u32,
}

impl MapManagerConfig {
    pub fn item(&self, item_type: u16) -> Option<&MapItemType> {
        self.item_types.iter().find(|t| t.item_type == item_type)
    }

    pub fn collectable_name(&self, item_type: u16) -> Option<u32> {
        self.collectable_names.iter().find(|(t, _)| *t == item_type).map(|(_, n)| *n)
    }
}

/// A filter of the world map (`Widget_MapFilters.Filter`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapFilter {
    /// `FilterName`: language id ("RACES").
    pub name: u32,
    /// `FilterType`: what is listed ([`map_filter`]).
    pub filter_type: u16,
}

/// The world map's `LiveMapManager_ScreenScript` (in `UI/SCREENS2/899991.BNDL`).
pub const LIVE_MAP_MANAGER_SCRIPT: u32 = 1_135_185;

/// A layout as a widget places it.
#[derive(Debug, Clone)]
pub struct PlacedLayout {
    /// Index of the widget in the screen (screens repeat names: five `Nitrous` variants).
    pub widget_index: usize,
    pub widget: String,
    pub widget_type: String,
    pub layout: Layout,
    pub instance: Option<LayoutInstance>,
}

impl AssetStore {
    /// The `Screen2` object of a screen bundle.
    pub fn ui_screen(&self, bundle: &str) -> Result<Screen> {
        fmt::ui::Screen::from_object(&self.ui_screen_object(bundle)?)
            .ok_or_else(|| Error::Format(fmt::Error::Malformed("not a screen".into())))
    }

    /// The decoded `Screen2` object of a screen bundle.
    pub fn ui_screen_object(&self, bundle: &str) -> Result<fmt::genesys::Object> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let res = b.load(e)?;
            // Screens are the objects whose type is `Screen2`; check the type before decoding everything.
            let Some(ty) = res.import_at(0) else { continue };
            if self.genesys_type(ty).is_some_and(|t| t.name == "Genesys.Gen.Screen2") {
                return self.decode_genesys(&res);
            }
        }
        Err(Error::Missing(ResourceId::NULL))
    }

    /// The points-of-interest looks a screen's `Widget_PointOfInterest` lists, with their layouts loaded
    /// (layouts that do not decode are left out).
    pub fn ui_poi_visuals(&self, bundle: &str) -> Result<Vec<PoiVisual>> {
        use fmt::genesys::Value;
        let screen = self.ui_screen_object(bundle)?;
        let widget = screen.find("Genesys.Gen.Widget_PointOfInterest").ok_or(Error::Missing(ResourceId::NULL))?.clone();
        let ids: Vec<ResourceId> = match widget.get(fmt::ui::POI_VISUALS_LIST) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::Handle(Some(id)) | Value::Ref(id) => Some(*id),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let placed = |l: &fmt::ui::WidgetLayout| -> Option<PlacedLayout> {
            let layout = self.ui_layout(l.layout?).ok()?;
            let instance = l.instance.and_then(|i| self.ui_layout_instance(i).ok());
            Some(PlacedLayout { widget_index: 0, widget: String::new(), widget_type: String::new(), layout, instance })
        };
        let mut out = Vec::new();
        for id in ids {
            let Some(v) = self.genesys_object(id).ok().and_then(|o| fmt::ui::PoiVisuals::from_object(&o)) else {
                continue;
            };
            out.push(PoiVisual {
                id: v.id,
                key: v.key,
                map: v.map.as_ref().and_then(placed),
                world: v.world.iter().filter_map(placed).collect(),
                face_camera: v.face_camera,
            });
        }
        Ok(out)
    }

    /// The point-of-interest keys the race code picks gate looks by (`FUN_007545b0`): `GameLogicConfig
    /// .CheckpointConfig` (`CheckPointsConfig`) and `.MapPoiConfig` in GLOBALCONFIG.BNDL.
    pub fn poi_keys(&self) -> Result<PoiKeys> {
        use fmt::genesys::{Value, field_hash};
        let b = self.bundle("GLOBALCONFIG.BNDL").ok_or(Error::Missing(ResourceId::NULL))?;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let Ok(root) = self.decode_genesys(&b.load(e)?) else { continue };
            let (Some(cp), Some(map)) =
                (root.find("Genesys.Gen.CheckPointsConfig"), root.find("Genesys.Gen.GameLogic.MapPoiConfig"))
            else {
                continue;
            };
            let key = |o: &fmt::genesys::Object, h: u32| match o.get(h) {
                Some(Value::UInt(u)) => *u as u32,
                Some(Value::Int(i)) => *i as u32,
                _ => 0,
            };
            let collectables = self.genesys_object(fmt::ui::object_id(crate::scoring::COLLECTABLES_CONFIG)).ok();
            let ck = |h: u32| collectables.as_ref().map_or(0, |o| key(o, h));
            return Ok(PoiKeys {
                gate: key(cp, field_hash("NormalPoi")),
                gate_preview: key(cp, 0xe9d2_21fe),
                finish: key(map, 0xf4d5_177c),
                finish_preview: key(map, 0x8d67_562d),
                player: key(map, 0x2940_1278),
                opponent: key(map, 0x5bfb_3553),
                body_shop: ck(0xd71d_ba55),
                jack_spot: ck(0x507e_9d87),
                jack_spot_owned: ck(0xa1b2_d86a),
                meetup: [key(map, 0x5626_fb85), key(map, 0x5e37_f18a)],
            });
        }
        Err(Error::Missing(ResourceId::NULL))
    }

    /// The world map's `LiveMapManager_ScreenScript` ([`LIVE_MAP_MANAGER_SCRIPT`]): its item types (icon and
    /// card keys per kind), the collectables' names and the racers' / cops' / route's looks.
    pub fn map_manager_config(&self) -> Result<MapManagerConfig> {
        use fmt::genesys::{Value, field_hash};
        let o = self.genesys_object(fmt::ui::object_id(LIVE_MAP_MANAGER_SCRIPT))?;
        let num = |o: &fmt::genesys::Object, h: u32| match o.get(h) {
            Some(Value::UInt(u)) => *u as u32,
            Some(Value::Int(i)) => *i as u32,
            _ => 0,
        };
        let structs = |h: u32| -> Vec<&fmt::genesys::Object> {
            match o.get(h) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| match v {
                        Value::Struct(s) => Some(s),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        let item_types = structs(field_hash("MapItemTypes"))
            .into_iter()
            .map(|t| MapItemType {
                item_type: num(t, field_hash("ItemType")) as u16,
                data_source: match t.get(field_hash("DataSource")) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => String::new(),
                },
                icon: num(t, 0x650f_3bfb),
                card: num(t, 0xc3f3_9ca3),
                options_height: num(t, field_hash("OptionsHeight")) as u16,
            })
            .collect();
        let collectable_names = structs(field_hash("CollectablesName"))
            .into_iter()
            .map(|c| (num(c, field_hash("ItemType")) as u16, num(c, field_hash("CollectableName"))))
            .collect();
        Ok(MapManagerConfig {
            item_types,
            collectable_names,
            selection_poi: num(&o, field_hash("SelectionPOI")),
            ai_player_poi: num(&o, field_hash("AIPlayerPOI")),
            ai_cop_poi: num(&o, field_hash("AICopPOI")),
            blacklist_driver_poi: num(&o, field_hash("BlackListDriver")),
            checkpoint_poi: num(&o, field_hash("CheckpointPOI")),
            finish_line_poi: num(&o, field_hash("FinishLinePOI")),
        })
    }

    /// The filters of the `Widget_MapFilters` in widget group `group` of a screen (the offline world map's
    /// 1135638: RACES, JACK SPOTS, RECOMMENDS, SPEED CAMERAS, BILLBOARDS).
    pub fn map_filters(&self, screen: &str, group: u32) -> Result<Vec<MapFilter>> {
        use fmt::genesys::{Value, field_hash};
        let o = self.ui_screen_object(screen)?;
        let mut groups = Vec::new();
        crate::frontend::collect(&o, "Genesys.Gen.WidgetGroup", &mut groups);
        let num = |o: &fmt::genesys::Object, h: u32| match o.get(h) {
            Some(Value::UInt(u)) => *u as u32,
            Some(Value::Int(i)) => *i as u32,
            _ => 0,
        };
        let g = groups
            .into_iter()
            .find(|g| num(g, 0x655d_3f5f) == group)
            .ok_or(Error::Missing(fmt::ui::object_id(group)))?;
        let mut widgets = Vec::new();
        crate::frontend::collect(g, "Genesys.Gen.Widget_MapFilters", &mut widgets);
        let w = widgets.first().ok_or(Error::Missing(fmt::ui::object_id(group)))?;
        let mut filters = Vec::new();
        crate::frontend::collect(w, "Genesys.Gen.Widget_MapFilters.Filter", &mut filters);
        Ok(filters
            .into_iter()
            .map(|f| MapFilter {
                name: num(f, field_hash("FilterName")),
                filter_type: num(f, field_hash("FilterType")) as u16,
            })
            .filter(|f| f.name != 0)
            .collect())
    }

    pub fn ui_layout(&self, id: ResourceId) -> Result<Layout> {
        Layout::from_object(&self.genesys_object(id)?)
            .ok_or_else(|| Error::Format(fmt::Error::Malformed(format!("{id} is not a UILayout"))))
    }

    pub fn ui_layout_instance(&self, id: ResourceId) -> Result<LayoutInstance> {
        LayoutInstance::from_object(&self.genesys_object(id)?)
            .ok_or_else(|| Error::Format(fmt::Error::Malformed(format!("{id} is not a UILayoutInstanceParams"))))
    }

    pub fn ui_text_style(&self, id: ResourceId) -> Result<TextStyle> {
        TextStyle::from_object(&self.genesys_object(id)?)
            .ok_or_else(|| Error::Format(fmt::Error::Malformed(format!("{id} is not a TextStyle"))))
    }

    /// Every layout of a screen's widgets (optionally only the named widgets), in widget order.
    pub fn ui_screen_layouts(&self, bundle: &str, widgets: Option<&[&str]>) -> Result<Vec<PlacedLayout>> {
        // Load the screen bundle first so its copies of shared resources are preferred.
        let screen = self.ui_screen(bundle)?;
        let mut out = Vec::new();
        for (widget_index, w) in screen.widgets.iter().enumerate() {
            if widgets.is_some_and(|names| !names.iter().any(|n| n.eq_ignore_ascii_case(&w.name))) {
                continue;
            }
            for l in &w.layouts {
                let Some(id) = l.layout else { continue };
                // A few layouts live only in bundles of other screens; skip what doesn't decode.
                let Ok(layout) = self.ui_layout(id) else { continue };
                let instance = l.instance.and_then(|i| self.ui_layout_instance(i).ok());
                out.push(PlacedLayout {
                    widget_index,
                    widget: w.name.clone(),
                    widget_type: w.type_name.clone(),
                    layout,
                    instance,
                });
            }
        }
        Ok(out)
    }

    /// A UI font by its bundle number (`UI/FONTS/<n>.BNDL`).
    pub fn ui_font(&self, number: u32) -> Result<fmt::Font> {
        let path = format!("UI/FONTS/{number}.BNDL");
        let b = self.bundle(&path).ok_or(Error::Missing(ResourceId(number as u64)))?;
        let e = b.entries_of_type(ResourceType::Font).next().ok_or(Error::Missing(ResourceId(number as u64)))?;
        let res = b.load(e)?;
        let tex = fmt::Font::texture_of(&res).ok_or(Error::Missing(ResourceId::NULL))?;
        let t = self.texture(tex)?;
        Ok(fmt::Font::parse(&res, [t.width, t.height])?)
    }

    /// UV rectangle `[u0, v0, u1, v1]` of a UI quad renderable.
    pub fn ui_quad_uv(&self, renderable: ResourceId) -> Option<[f32; 4]> {
        let r = self.renderable(renderable).ok()?;
        let mesh = r.meshes.first()?;
        fmt::ui::quad_uv(r.vertex_bytes(mesh).ok()?)
    }

    /// UV rectangle of an element's image (first renderable), the whole texture when unknown.
    pub fn ui_element_uv(&self, e: &Element) -> [f32; 4] {
        e.renderables.first().and_then(|&r| self.ui_quad_uv(r)).unwrap_or([0.0, 0.0, 1.0, 1.0])
    }
}

/// Cache of text styles and fonts for one UI build.
#[derive(Default)]
pub struct UiCache {
    pub styles: HashMap<ResourceId, Option<TextStyle>>,
    pub fonts: HashMap<u32, Option<fmt::Font>>,
}

impl UiCache {
    pub fn style(&mut self, assets: &AssetStore, id: ResourceId) -> Option<&TextStyle> {
        self.styles.entry(id).or_insert_with(|| assets.ui_text_style(id).ok()).as_ref()
    }

    pub fn font(&mut self, assets: &AssetStore, number: u32) -> Option<&fmt::Font> {
        self.fonts.entry(number).or_insert_with(|| assets.ui_font(number).ok()).as_ref()
    }
}

/// How a UI element blends, from its material's shader (`UIShader`, `UIAdditiveShader`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiBlend {
    #[default]
    Alpha,
    Additive,
}

impl AssetStore {
    /// Shader name of a `UIMaterial` (its `Material` field names a render material by UniqueId).
    pub fn ui_material_shader(&self, ui_material: ResourceId) -> Option<String> {
        let o = self.genesys_object(ui_material).ok()?;
        let id = match o.get(fmt::genesys::field_hash("Material"))? {
            fmt::genesys::Value::Int(i) => *i as u32,
            fmt::genesys::Value::UInt(u) => *u as u32,
            _ => return None,
        };
        let m = self.material(fmt::ui::object_id(id)).ok()?;
        Some(self.shader(m.shader?).ok()?.name)
    }

    pub fn ui_blend(&self, ui_material: Option<ResourceId>) -> UiBlend {
        match ui_material.and_then(|m| self.ui_material_shader(m)) {
            Some(s) if s.contains("Additive") => UiBlend::Additive,
            _ => UiBlend::Alpha,
        }
    }
}

/// `UIColourPalette` of `UI/UICONFIG.BNDL` (the named colours behaviours blend to).
pub const UI_COLOUR_PALETTE: ResourceId = ResourceId(0x0100_0000_0001_72aa);

/// Everything needed to draw a set of layouts, resolved from the assets once (on a loader thread), so
/// drawing a frame needs no asset access.
#[derive(Debug, Clone, Default)]
pub struct UiScene {
    pub layouts: Vec<PlacedLayout>,
    pub styles: HashMap<ResourceId, TextStyle>,
    pub fonts: HashMap<u32, fmt::Font>,
    /// Language strings the labels name.
    pub strings: HashMap<u32, String>,
    /// UV rectangle of each quad renderable.
    pub uvs: HashMap<ResourceId, [f32; 4]>,
    /// Blend mode of each `UIMaterial`.
    pub blends: HashMap<ResourceId, UiBlend>,
    /// Layouts that element stacks place (cell contents), by resource ID.
    pub cells: HashMap<ResourceId, fmt::ui::Layout>,
    /// Units for the code-side formatters (`DIST`: metres in, metric or imperial out); `None` = raw numbers.
    pub metric: Option<bool>,
    /// A menu screen ([`UiFrame::letterbox`]: the whole frame fitted, no HUD correction, black bars) rather
    /// than a HUD screen ([`UiFrame::new`]).
    pub letterbox: bool,
    /// The UI's named colours that colour steps blend to ([`AssetStore::ui_colour_palette`]).
    pub palette: HashMap<String, [f32; 4]>,
}

fn walk_elements<'a>(elements: impl IntoIterator<Item = &'a Element>, f: &mut impl FnMut(&'a Element)) {
    for e in elements {
        f(e);
        if let Some(s) = &e.sub_image {
            walk_elements(std::iter::once(&**s), f);
        }
    }
}

impl AssetStore {
    /// Resolve the styles, fonts, strings, UVs and blend modes the layouts use.
    /// The UI's named colours (`UIColourPalette` [`UI_COLOUR_PALETTE`] in `UI/UICONFIG.BNDL`, 179 `UIColour`
    /// {`263d15bc` name, `Colour` RGBA bytes}), as linear RGBA like element tints (sRGB bytes decoded: `WHITE`
    /// ebebeb = 0.8308, the tint of white text). Empty when the object is missing.
    pub fn ui_colour_palette(&self) -> HashMap<String, [f32; 4]> {
        use fmt::genesys::{Value, field_hash};
        let Ok(o) = self.genesys_object(UI_COLOUR_PALETTE) else { return HashMap::new() };
        let Some(Value::Array(entries)) = o.get(0xd46f_dc01) else { return HashMap::new() };
        let linear = |b: u8| {
            let c = b as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let mut out = HashMap::new();
        for v in entries {
            let Value::Struct(c) = v else { continue };
            let (Some(Value::Str(name)), Some(Value::Raw(rgba))) = (c.get(0x263d_15bc), c.get(field_hash("Colour")))
            else {
                continue;
            };
            if let [r, g, b, a, ..] = rgba[..] {
                out.insert(name.clone(), [linear(r), linear(g), linear(b), a as f32 / 255.0]);
            }
        }
        out
    }

    pub fn ui_scene(&self, layouts: Vec<PlacedLayout>, lang: &fmt::Language) -> UiScene {
        let mut scene = UiScene { layouts, palette: self.ui_colour_palette(), ..Default::default() };
        let mut cache = UiCache::default();
        let mut all = Vec::new();
        for pl in &scene.layouts {
            walk_elements(&pl.layout.elements, &mut |e| all.push(e.clone()));
        }
        // Element stacks' cell layouts, recursively.
        let mut i = 0;
        while i < all.len() {
            if let Some(st) = all[i].stack.clone() {
                let ids =
                    st.header.iter().chain(&st.default).chain(&st.footer).chain(st.templates.iter().flat_map(|t| &t.1));
                for &id in ids {
                    if scene.cells.contains_key(&id) {
                        continue;
                    }
                    // A missing cell layout draws nothing (the stack's other cells still do).
                    if let Ok(l) = self.ui_layout(id) {
                        walk_elements(&l.elements, &mut |e| all.push(e.clone()));
                        scene.cells.insert(id, l);
                    }
                }
            }
            i += 1;
        }
        for e in &all {
            for &r in &e.renderables {
                if let std::collections::hash_map::Entry::Vacant(v) = scene.uvs.entry(r)
                    && let Some(uv) = self.ui_quad_uv(r)
                {
                    v.insert(uv);
                }
            }
            if let Some(m) = e.material {
                scene.blends.entry(m).or_insert_with(|| self.ui_blend(Some(m)));
            }
            let Some(t) = &e.text else { continue };
            if t.string_id != 0
                && let Some(s) = lang.get(t.string_id)
            {
                scene.strings.insert(t.string_id, s.to_string());
            }
            let Some(style_id) = t.style else { continue };
            let Some(style) = cache.style(self, style_id).cloned() else { continue };
            if let Some(font) = cache.font(self, style.font).cloned() {
                scene.fonts.insert(style.font, font);
            }
            scene.styles.insert(style_id, style);
        }
        scene
    }
}

impl UiScene {
    /// Every texture the scene draws: element images and font atlases.
    pub fn textures(&self) -> Vec<ResourceId> {
        let mut out = Vec::new();
        for pl in &self.layouts {
            walk_elements(&pl.layout.elements, &mut |e| out.extend(e.texture));
        }
        for l in self.cells.values() {
            walk_elements(&l.elements, &mut |e| out.extend(e.texture));
        }
        out.extend(self.fonts.values().filter_map(|f| f.texture));
        out.sort();
        out.dedup();
        out
    }
}

/// A textured (or solid, `texture: None`) quad in screen pixels, in draw order.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub texture: Option<ResourceId>,
    /// `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// `[x, y, width, height]`, screen pixels.
    pub rect: [f32; 4],
    /// Degrees clockwise about the rectangle's centre.
    pub rotation: f32,
    /// Linear RGBA (tint × opacity).
    pub colour: [f32; 4],
    pub blend: UiBlend,
    /// The minimap goes here (drawn by the caller).
    pub map: bool,
}

/// Binding lookup: `(path, [argument])` → value, `None` when the game state isn't known.
pub type Lookup<'a> = dyn FnMut(&str, Option<&str>) -> Option<fmt::binding::BindValue> + 'a;

fn eval(binding: &str, lookup: &mut Lookup) -> Option<fmt::binding::BindValue> {
    fmt::binding::Binding::parse(binding)?.eval(lookup)
}

fn eval_num(binding: &str, lookup: &mut Lookup) -> Option<f32> {
    eval(binding, lookup).map(|v| v.num() as f32)
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// How the game maps its 1280×720 UI frame to a screen, and its `UIConfig.HudAspectCorrection` for that screen.
///
/// `FUN_0078faa0` (run when the back buffer's size changes, `FUN_0079a810`) with the screen's aspect a = w / h:
/// for 1 ≤ a ≤ 8/3 the frame keeps its height (720 reference pixels fill the screen's height) and `Left` =
/// (1280 − 720·a) / 2 + 0.5, `Right` = −`Left`: at 4:3 the visible frame is x 160..1120 and left-side HUD
/// layouts move in by 160, at 21:9 out by 200. For a > 8/3 (three monitors) the HUD stays in the centre 16:9
/// (`Left` = 0.5). For a < 1 (portrait) the frame keeps its width and `Top` = (720 − 1280 / a) / 2 + 0.5,
/// `Bottom` = −`Top`. `ScreenWidthMultiplier` = min(a, 16/9 above 8/3) · 0.5625 and `EffectiveScreenWidth` =
/// 1280 × that (`FUN_00794700`, per frame: "the width of a 16:9 screen with no margin scaling is 1280"). The
/// options' screen-scale margins (`FUN_00794700` adds them; widget `ScreenScale` of screen 469958) are taken as
/// 0, the default (model decision until a live read). The frame itself is centred on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiFrame {
    /// Screen pixels per reference pixel.
    pub scale: f32,
    /// Screen position of the frame's (0, 0).
    pub origin: [f32; 2],
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub width_multiplier: f32,
    pub effective_width: f32,
}

impl UiFrame {
    pub fn new(screen: [f32; 2]) -> Self {
        let [w, h] = [screen[0].max(1.0), screen[1].max(1.0)];
        let r = fmt::ui::REFERENCE_SIZE;
        let a = w / h;
        // Above 8/3 the code computes with the width of a 16:9 screen of the same height.
        let a_hud = if a > 8.0 / 3.0 { 16.0 / 9.0 } else { a };
        let mult = a_hud * 0.5625;
        let (left, top) =
            if a < 1.0 { (0.0, (r[1] - r[0] / w * h) * 0.5 + 0.5) } else { ((r[0] - r[1] * a_hud) * 0.5 + 0.5, 0.0) };
        // Portrait windows fit the frame's width, every other one its height.
        let scale = if a < 1.0 { w / r[0] } else { h / r[1] };
        Self {
            scale,
            origin: [w * 0.5 - r[0] * 0.5 * scale, h * 0.5 - r[1] * 0.5 * scale],
            left,
            right: -left,
            top,
            bottom: -top,
            width_multiplier: mult,
            effective_width: r[0] * mult,
        }
    }

    /// A menu screen's frame (the UI manager's byte +0x2ecf4 set: `FUN_00794700` leaves the corrections at 0 and
    /// `FUN_006240b0` draws black bars outside the frame): the whole 1280×720 frame fitted inside the screen and
    /// centred. Captures of the original at 1024×768 and 1680×720 (`ref/ui-2026-10-19/`): the title, the boot
    /// transition, the pause menu, the settings and the map are letterboxed / pillarboxed so; the driving HUD
    /// and Easydrive are not ([`UiFrame::new`]).
    pub fn letterbox(screen: [f32; 2]) -> Self {
        let [w, h] = [screen[0].max(1.0), screen[1].max(1.0)];
        let r = fmt::ui::REFERENCE_SIZE;
        let scale = (w / r[0]).min(h / r[1]);
        Self {
            scale,
            origin: [(w - r[0] * scale) * 0.5, (h - r[1] * scale) * 0.5],
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            width_multiplier: 1.0,
            effective_width: r[0],
        }
    }

    /// The black bars outside the frame (`FUN_006240b0`), for menu screens: up to four quads, drawn last.
    pub fn bars(&self, screen: [f32; 2]) -> Vec<Quad> {
        let [x, y, w, h] = self.rect();
        let bar = |rect: [f32; 4]| Quad {
            texture: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            rect,
            rotation: 0.0,
            colour: [0.0, 0.0, 0.0, 1.0],
            blend: UiBlend::Alpha,
            map: false,
        };
        [
            [0.0, 0.0, screen[0], y],
            [0.0, y + h, screen[0], screen[1] - y - h],
            [0.0, y, x, h],
            [x + w, y, screen[0] - x - w, h],
        ]
        .into_iter()
        .filter(|r| r[2] > 0.5 && r[3] > 0.5)
        .map(bar)
        .collect()
    }

    /// The frame scaled to cover the whole screen (centred, keeping its aspect): for full-screen backdrops of
    /// HUD screens. Equal to the frame from 1:1 up to 16:9.
    pub fn cover(&self) -> Self {
        let r = fmt::ui::REFERENCE_SIZE;
        let screen = [self.origin[0] * 2.0 + r[0] * self.scale, self.origin[1] * 2.0 + r[1] * self.scale];
        let scale = (screen[0] / r[0]).max(screen[1] / r[1]);
        Self { scale, origin: [(screen[0] - r[0] * scale) * 0.5, (screen[1] - r[1] * scale) * 0.5], ..*self }
    }

    /// Screen rectangle `[x, y, w, h]` of the whole frame.
    pub fn rect(&self) -> [f32; 4] {
        let r = fmt::ui::REFERENCE_SIZE;
        [self.origin[0], self.origin[1], r[0] * self.scale, r[1] * self.scale]
    }

    /// Screen position of a frame point (reference pixels).
    pub fn to_screen(&self, p: [f32; 2]) -> [f32; 2] {
        [self.origin[0] + p[0] * self.scale, self.origin[1] + p[1] * self.scale]
    }

    /// The `UIConfig.HudAspectCorrection.<field>` binding's value.
    pub fn correction(&self, field: &str) -> Option<f32> {
        Some(match field {
            "Left" => self.left,
            "Right" => self.right,
            "Top" => self.top,
            "Bottom" => self.bottom,
            "ScreenWidthMultiplier" => self.width_multiplier,
            "EffectiveScreenWidth" => self.effective_width,
            _ => return None,
        })
    }
}

/// Places one layout: its anchor point (`AnchorType`, a point of its own 1280×720 authoring frame) goes to the
/// sum of its instance's translate components, the bound ones (`UIConfig.HudAspectCorrection.Left(AXES:X)`)
/// evaluated and put on their axis; then the frame goes to the screen ([`UiFrame`]).
#[derive(Debug, Clone, Copy)]
struct Placement {
    frame: UiFrame,
    /// Added to a layout point to get its frame point.
    shift: [f32; 2],
}

impl Placement {
    fn new(instance: Option<&LayoutInstance>, frame: UiFrame, lookup: &mut Lookup) -> Self {
        let shift = match instance {
            Some(i) => {
                let a = i.anchor_point();
                let mut t = i
                    .transforms
                    .iter()
                    .find(|t| t.kind == 1 && t.binding.is_empty())
                    .map_or(a, |t| [t.value[0], t.value[1]]);
                // Bound scalar translates on one axis (`(AXES:X)` / `(AXES:Y)`); vector bindings such as the
                // minimap's `MapOrigin` are left to their widgets.
                for c in i.transforms.iter().filter(|t| t.kind == 1 && t.binding.contains("AXES:")) {
                    let Some(v) = eval_num(&c.binding, lookup) else { continue };
                    let axis = usize::from(c.binding.contains("AXES:Y"));
                    t[axis] += v + c.value[axis];
                }
                [t[0] - a[0], t[1] - a[1]]
            }
            None => [0.0; 2],
        };
        Self { frame, shift }
    }

    fn point(&self, p: [f32; 2]) -> [f32; 2] {
        self.frame.to_screen([p[0] + self.shift[0], p[1] + self.shift[1]])
    }

    fn rect(&self, r: [f32; 4]) -> [f32; 4] {
        let p = self.point([r[0], r[1]]);
        [p[0], p[1], r[2] * self.frame.scale, r[3] * self.frame.scale]
    }
}

/// A label's text with its bindings filled in; private-use formatting codes (`U+F000`–`U+F0FF`) dropped.
fn label_text(scene: &UiScene, e: &Element, lookup: &mut Lookup) -> String {
    let Some(t) = &e.text else { return String::new() };
    // A known `DisplayText` binding replaces the authored string ("PLACEHOLDER" under
    // `Widget.HUD.SummaryHeadline`); when the binding's value is unknown the string stays.
    let bound = (!e.bindings.text.is_empty())
        .then(|| fmt::binding::Binding::parse(&e.bindings.text))
        .flatten()
        .and_then(|b| b.eval(lookup).map(|v| b.format_in(&v, scene.metric)));
    if let Some(text) = bound {
        return text.chars().filter(|c| !('\u{f000}'..='\u{f0ff}').contains(c)).collect();
    }
    let template = match scene.strings.get(&t.string_id) {
        Some(s) if t.string_id != 0 => s.clone(),
        _ if !e.bindings.text.is_empty() => format!("{{{}}}", e.bindings.text),
        _ => return String::new(),
    };
    let text = fmt::ui::substitute(&template, |expr| {
        let b = fmt::binding::Binding::parse(expr)?;
        let v = b.eval(lookup)?;
        Some(b.format_in(&v, scene.metric))
    });
    text.chars().filter(|c| !('\u{f000}'..='\u{f0ff}').contains(c)).collect()
}

impl UiScene {
    /// Draw every layout for a screen of `screen` pixels. Elements whose visibility binding is unknown or
    /// false are skipped; masks clip (axis-aligned, by rectangle) the elements up to their `PopMask`.
    /// Triggered behaviours show their settled state (see [`UiAnim`]).
    pub fn draw(&self, screen: [f32; 2], lookup: &mut Lookup) -> Vec<Quad> {
        self.draw_with(screen, lookup, &mut DrawOptions::default())
    }

    /// [`draw`](Self::draw) with options: only some layouts, and triggered behaviours animated over time.
    pub fn draw_with(&self, screen: [f32; 2], lookup: &mut Lookup, options: &mut DrawOptions) -> Vec<Quad> {
        let frame = if self.letterbox { UiFrame::letterbox(screen) } else { UiFrame::new(screen) };
        // `UIConfig.HudAspectCorrection.*` comes from the screen ([`UiFrame`]).
        let lookup = &mut |path: &str, arg: Option<&str>| -> Option<fmt::binding::BindValue> {
            if let Some(field) = path.strip_prefix("UIConfig.HudAspectCorrection.") {
                return frame.correction(field).map(|v| f64::from(v).into());
            }
            lookup(path, arg)
        };
        let mut out = Vec::new();
        for (index, pl) in self.layouts.iter().enumerate() {
            if options.layouts.as_ref().is_some_and(|only| !only(index)) {
                continue;
            }
            if let Some(i) = &pl.instance
                && !i.visibility.is_empty()
                && !eval(&i.visibility, lookup).is_some_and(|v| v.truthy())
            {
                continue;
            }
            options.layout = index as u32;
            options.cell = u32::MAX;
            options.replacement = pl.instance.as_ref().map_or(0, |i| i.replacement_index);
            let place = Placement::new(pl.instance.as_ref(), frame, lookup);
            let mut clips: Vec<[f32; 4]> = Vec::new();
            for e in &pl.layout.elements {
                if e.kind == ElementKind::Mask {
                    if e.pop_mask {
                        clips.pop();
                    } else {
                        let (rect, _) = element_rect(e, lookup);
                        let r = place.rect(rect);
                        clips.push(clips.last().map_or(r, |&c| intersect(c, r)));
                    }
                    continue;
                }
                self.element(e, &place, None, clips.last().copied(), lookup, options, &mut out);
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn element(
        &self,
        e: &Element,
        place: &Placement,
        parent: Option<[f32; 2]>,
        clip: Option<[f32; 4]>,
        lookup: &mut Lookup,
        options: &mut DrawOptions,
        out: &mut Vec<Quad>,
    ) {
        if !e.bindings.visibility.is_empty() && !eval(&e.bindings.visibility, lookup).is_some_and(|v| v.truthy()) {
            return;
        }
        let tween = options.tween(e, lookup, &self.palette);
        if tween.opacity <= 0.0 {
            return;
        }
        let (mut rect, mut pivot) = element_rect(e, lookup);
        if tween.scale != 1.0 {
            let (w, h) = (rect[2] * tween.scale, rect[3] * tween.scale);
            rect = [rect[0] + (rect[2] - w) * 0.5, rect[1] + (rect[3] - h) * 0.5, w, h];
        }
        rect[0] += tween.offset[0];
        rect[1] += tween.offset[1];
        pivot = [pivot[0] + tween.offset[0], pivot[1] + tween.offset[1]];
        // Sub-elements (backing images) are placed relative to their label.
        if let Some(p) = parent {
            rect[0] += p[0];
            rect[1] += p[1];
            pivot = [pivot[0] + p[0], pivot[1] + p[1]];
        }
        let rotation = eval_num(&e.bindings.rotation, lookup).unwrap_or(e.rotation) + tween.rotation;
        let opacity = eval_num(&e.bindings.opacity, lookup).map_or(e.opacity, |o| o.clamp(0.0, 1.0)) * tween.opacity;
        let blend = e.material.and_then(|m| self.blends.get(&m).copied()).unwrap_or_default();
        if let Some(st) = &e.stack {
            self.stack(st, place, [rect[0], rect[1]], clip, lookup, options, out);
            return;
        }
        // On HUD screens, full-screen backdrops (a solid shape or a streamed picture at least as large as the
        // frame: fades, flashes) cover the whole window whatever its aspect (model decision: the height-fitted
        // frame alone leaves bars beside them above 16:9). Menu screens letterbox instead, as the original.
        let cover;
        let backdrop = e.kind == ElementKind::Shape || (e.kind == ElementKind::Image && !e.bindings.image.is_empty());
        let place = if parent.is_none()
            && !self.letterbox
            && backdrop
            && rect[2] >= fmt::ui::REFERENCE_SIZE[0] - 0.5
            && rect[3] >= fmt::ui::REFERENCE_SIZE[1] - 0.5
        {
            cover = Placement { frame: place.frame.cover(), shift: place.shift };
            &cover
        } else {
            place
        };
        let screen_rect = place.rect(rect);
        let screen_pivot = place.point(pivot);
        // Backed labels: the backing image first, filling the label's rectangle.
        if e.kind.has_text()
            && let Some(sub) = &e.sub_image
        {
            let mut backing = (**sub).clone();
            backing.size = [rect[2], rect[3]];
            backing.position = [0.0, 0.0];
            backing.anchor = 0;
            backing.opacity *= tween.opacity;
            self.element(&backing, place, Some([rect[0], rect[1]]), clip, lookup, options, out);
        }
        let mut push = |texture: Option<ResourceId>, uv: [f32; 4], r: [f32; 4], colour: [f32; 4], map: bool| {
            push_quad(out, Quad { texture, uv, rect: r, rotation, colour, blend, map }, screen_pivot, clip);
        };
        let c = tween.colour.unwrap_or(e.tint);
        let tint = [c[0], c[1], c[2], c[3] * opacity];
        match e.kind {
            ElementKind::Image | ElementKind::SubImage => {
                // `ImageBinding`: the lookup answers with a texture resource id (16 hex digits) to show whole
                // (streamed pictures such as the loading screen's `Transitions.Image#`, `#` being the layout
                // instance's replacement index).
                let bound = (!e.bindings.image.is_empty())
                    .then(|| e.bindings.image.replace('#', &options.replacement.to_string()))
                    .and_then(|b| eval(&b, lookup))
                    .and_then(|v| match v {
                        fmt::binding::BindValue::Text(s) if s.trim().len() == 16 => ResourceId::parse(s.trim()),
                        fmt::binding::BindValue::Text(_) => None,
                        fmt::binding::BindValue::Num(_) => None,
                    });
                let Some(tex) = bound.or(e.texture) else { return };
                let mut uv = match bound {
                    Some(_) => [0.0, 0.0, 1.0, 1.0],
                    None => {
                        e.renderables.first().and_then(|r| self.uvs.get(r)).copied().unwrap_or([0.0, 0.0, 1.0, 1.0])
                    }
                };
                if e.flip[0] {
                    uv.swap(0, 2);
                }
                if e.flip[1] {
                    uv.swap(1, 3);
                }
                push(Some(tex), uv, screen_rect, tint, false);
            }
            ElementKind::Shape => {
                let bg = e.background.unwrap_or([1.0; 4]);
                push(None, [0.0, 0.0, 1.0, 1.0], screen_rect, [bg[0], bg[1], bg[2], bg[3] * opacity], false);
            }
            ElementKind::Map => push(None, [0.0, 0.0, 1.0, 1.0], screen_rect, [1.0, 1.0, 1.0, opacity], true),
            k if k.has_text() => self.text(e, rect, opacity, place, &mut push, lookup),
            _ => {}
        }
    }

    fn text(
        &self,
        e: &Element,
        rect: [f32; 4],
        opacity: f32,
        place: &Placement,
        push: &mut impl FnMut(Option<ResourceId>, [f32; 4], [f32; 4], [f32; 4], bool),
        lookup: &mut Lookup,
    ) {
        let Some(t) = &e.text else { return };
        let Some(style) = t.style.and_then(|s| self.styles.get(&s)) else { return };
        let Some(font) = self.fonts.get(&style.font) else { return };
        let Some(atlas) = font.texture else { return };
        let text = label_text(self, e, lookup);
        if text.trim().is_empty() {
            return;
        }
        let inner = [
            rect[0] + e.margins[0],
            rect[1] + e.margins[1],
            rect[2] - e.margins[0] - e.margins[2],
            rect[3] - e.margins[1] - e.margins[3],
        ];
        let size = style.size;
        // In the font's units, scaled with the glyphs: the odometer (size 19) and the speed (size 38) share spacing
        // −5, and the original's odometer digits do not overlap (`ref/ui-2026-10-19/hud_1024x768.png`).
        let spacing = style.character_spacing * size / font.em.max(1.0);
        let lines: Vec<_> = text.split('\n').map(|l| font.layout(l, size, spacing)).collect();
        let line_h = size + style.line_spacing;
        let total_h = line_h * lines.len() as f32 - style.line_spacing;
        let colour = [style.colour[0], style.colour[1], style.colour[2], style.colour[3] * opacity];
        let [aw, ah] = font.atlas_size;
        for (i, (quads, width)) in lines.iter().enumerate() {
            let x0 = match t.justification {
                1 => inner[0] + (inner[2] - width) * 0.5,
                2 => inner[0] + inner[2] - width,
                _ => inner[0],
            };
            // Vertically the text sits by the element's anchor row: top-anchored labels at the top of their box
            // (the pause title, the speed and its unit), middle ones centred (menu rows), bottom ones at the
            // bottom. Our reading of captures of the original (research log 2026-10-19 "UI frame").
            static TEXT_LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *TEXT_LOG.get_or_init(|| std::env::var_os("MW_UI_TEXT_LOG").is_some()) {
                eprintln!(
                    "ui text {text:?}: font {} size {size} spacing {spacing} box {inner:?} width {width} quads {:?}",
                    style.font,
                    quads.iter().map(|q| q.rect).collect::<Vec<_>>()
                );
            }
            let free = inner[3] - total_h;
            let y0 = inner[1] + free * (e.anchor / 3).min(2) as f32 * 0.5 + i as f32 * line_h;
            for q in quads {
                let r = place.rect([x0 + q.rect[0], y0 + q.rect[1], q.rect[2], q.rect[3]]);
                let a = q.atlas;
                // Key boxes: a light box with the key's name dark, whatever the label's colour (the prompt
                // strings' `U+F040` / `U+F092` codes are not decoded; the prompt labels' light and dark, model
                // decision).
                let ink = match q.key {
                    1 => [KEY_BOX[0], KEY_BOX[1], KEY_BOX[2], colour[3]],
                    2 => [KEY_INK[0], KEY_INK[1], KEY_INK[2], colour[3]],
                    _ => colour,
                };
                push(Some(atlas), [a[0] / aw, a[1] / ah, (a[0] + a[2]) / aw, (a[1] + a[3]) / ah], r, ink, false);
            }
        }
    }
}

/// Width of a cell of a stack running across: the rightmost edge of the elements that size the cell.
fn cell_width(layout: &Layout) -> f32 {
    sizing(layout).map(|e| e.top_left()[0] + e.size[0]).fold(0.0, f32::max)
}

/// The elements that size a stack cell (`Element::sizes_cell`), or every element when none is flagged.
fn sizing(layout: &Layout) -> impl Iterator<Item = &Element> {
    let any = layout.elements.iter().any(|e| e.sizes_cell);
    layout.elements.iter().filter(move |e| e.sizes_cell || !any)
}

/// Height of a stack cell (reference pixels): the lowest bottom of the elements that size it, from the cell's
/// top. A nested stack counts its own `Height`, or its default cell's when that is 0 (the selected Easydrive row
/// is a 25 px stack whose glow images reach lower). The pause menu's rows come out 40 px (52 px selected) as in
/// the original (`ref/frontend-2026-10-10/o4.png`); Easydrive's are 26 px against the 26.42 px per item the
/// `EasyDrivePanel` moves by (`Widget.EasyDrive.ItemsAbovePanel(MULT:26.42)`): `CellSizeDefinition` and
/// `Padding` (empty / 0 there) are not decoded.
fn cell_height(scene: &UiScene, layout: &Layout, depth: u32, lookup: &mut Lookup) -> f32 {
    let mut h: f32 = 0.0;
    for e in sizing(layout) {
        let own = match &e.stack {
            Some(st) if depth < 8 => {
                let inner = st.default.first().and_then(|id| scene.cells.get(id));
                let one = if e.size[1] > 0.0 {
                    e.size[1]
                } else {
                    inner.map_or(0.0, |l| cell_height(scene, l, depth + 1, lookup))
                };
                // A nested list grows by one cell per entry (the tier-history titles above Easydrive's rows).
                let n = if st.length.is_empty() {
                    1.0
                } else {
                    // An empty list takes no room (Easydrive's root tier has no title: its first row sits right
                    // under the prompt, `ref/ui-2026-10-19/easydrive_1680x720.png`).
                    eval_num(&st.length, lookup).map_or(1.0, |v| v.clamp(0.0, 64.0).floor())
                };
                one * n
            }
            _ => e.size[1],
        };
        h = h.max(e.top_left()[1] + own);
    }
    h
}

impl UiScene {
    /// Draw an element stack: header cells, one cell per data-source entry, footer cells, top to bottom from
    /// `origin` (reference pixels). Inside a cell, `#` and `#parent_index#` in binding arguments are the
    /// entry's index and the enclosing stack's index, a binding without a dot (`Name`, `Locked`) is a field of
    /// the entry and is looked up as `<data source>[<index>].<field>`, and the lookup answers `#index` with
    /// the current index (how a nested stack learns `#parent_index#`).
    #[allow(clippy::too_many_arguments)]
    fn stack(
        &self,
        st: &fmt::ui::Stack,
        place: &Placement,
        origin: [f32; 2],
        clip: Option<[f32; 4]>,
        lookup: &mut Lookup,
        options: &mut DrawOptions,
        out: &mut Vec<Quad>,
    ) {
        let parent = lookup("#index", None).map_or(0, |v| v.num() as i64);
        let list = !st.length.is_empty();
        let n = if list { eval_num(&st.length, lookup).map_or(0, |v| v.clamp(0.0, 64.0) as usize) } else { 1 };
        let mut source = st.data_source.trim_start_matches('~').replace("#parent_index#", &parent.to_string());
        // A nested stack over `{parent}` lists the enclosing stack's current item (the pause menu's rows).
        if source.contains("{parent}")
            && let Some(fmt::binding::BindValue::Text(item)) = lookup("#item", None)
        {
            source = source.replace("{parent}", &item);
        }
        // Cells follow each other down, or across for `direction` 0 (the navigation tabs).
        let across = st.direction == 0 && list;
        let mut y = if across { origin[0] } else { origin[1] };
        let mut cell = |this: &Self,
                        ids: &[ResourceId],
                        index: Option<usize>,
                        y: &mut f32,
                        lookup: &mut Lookup,
                        options: &mut DrawOptions| {
            let i = index.unwrap_or(0);
            let item = match (index, list) {
                (Some(i), true) => format!("{source}[{i}]"),
                _ => source.clone(),
            };
            let mut inner = |path: &str, arg: Option<&str>| -> Option<fmt::binding::BindValue> {
                if path == "#index" {
                    return Some((i as f64).into());
                }
                if path == "#item" {
                    return Some(fmt::binding::BindValue::Text(item.clone()));
                }
                let arg = arg.map(|a| a.replace("#parent_index#", &parent.to_string()).replace('#', &i.to_string()));
                if !path.contains('.') && !item.is_empty() {
                    return lookup(&format!("{item}.{path}"), arg.as_deref());
                }
                lookup(path, arg.as_deref())
            };
            let mut h: f32 = 0.0;
            for l in ids.iter().filter_map(|id| this.cells.get(id)) {
                for e in &l.elements {
                    if e.kind != ElementKind::Mask {
                        // Behaviour state per cell: entry index + 1 (0 = header / footer).
                        let outer = options.cell;
                        options.cell = index.map_or(0, |i| i as u32 + 1);
                        let at = if across { [*y, origin[1]] } else { [origin[0], *y] };
                        this.element(e, place, Some(at), clip, &mut inner, options, out);
                        options.cell = outer;
                    }
                }
                h = h.max(if across { cell_width(l) } else { cell_height(this, l, 0, &mut inner) });
            }
            *y += h;
        };
        cell(self, &st.header, None, &mut y, lookup, options);
        for i in 0..n {
            // The first template whose binding holds for this entry, else the default.
            let ids = st
                .templates
                .iter()
                .find(|(b, _)| {
                    let b = b.replace("#parent_index#", &parent.to_string()).replace('#', &i.to_string());
                    !b.is_empty() && eval(&b, lookup).is_some_and(|v| v.truthy())
                })
                .map_or(&st.default, |t| &t.1);
            cell(self, ids, Some(i), &mut y, lookup, options);
        }
        cell(self, &st.footer, None, &mut y, lookup, options);
    }
}

fn element_rect(e: &Element, lookup: &mut Lookup) -> ([f32; 4], [f32; 2]) {
    let x = eval_num(&e.bindings.position_x, lookup).unwrap_or(e.position[0]);
    let y = eval_num(&e.bindings.position_y, lookup).unwrap_or(e.position[1]);
    let w = e.size[0] * eval_num(&e.bindings.width_scale, lookup).unwrap_or(1.0);
    let h = e.size[1] * eval_num(&e.bindings.height_scale, lookup).unwrap_or(1.0);
    let (col, row) = ((e.anchor % 3) as f32, (e.anchor / 3).min(2) as f32);
    ([x - w * col * 0.5, y - h * row * 0.5, w, h], [x, y])
}

/// Rotate a quad about `pivot` (screen pixels) and clip it to `clip`, then append it.
fn push_quad(out: &mut Vec<Quad>, mut q: Quad, pivot: [f32; 2], clip: Option<[f32; 4]>) {
    // The map turns its content (heading-up), not its frame.
    if q.map {
        q.rotation = 0.0;
    }
    let r = &mut q.rect;
    if q.colour[3] <= 0.001 || r[2] <= 0.0 || r[3] <= 0.0 {
        return;
    }
    if q.rotation != 0.0 {
        // Rotate about the pivot: move the centre, then spin the quad about its own centre.
        let (s, c) = q.rotation.to_radians().sin_cos();
        let (cx, cy) = (r[0] + r[2] * 0.5 - pivot[0], r[1] + r[3] * 0.5 - pivot[1]);
        let (nx, ny) = (pivot[0] + cx * c - cy * s, pivot[1] + cx * s + cy * c);
        r[0] = nx - r[2] * 0.5;
        r[1] = ny - r[3] * 0.5;
    }
    if let Some(c) = clip {
        let k = intersect(c, *r);
        if k[2] <= 0.0 || k[3] <= 0.0 {
            return;
        }
        // Rotated quads aren't clipped exactly (only dropped when fully outside).
        if q.rotation == 0.0 {
            let (du, dv) = ((q.uv[2] - q.uv[0]) / r[2], (q.uv[3] - q.uv[1]) / r[3]);
            q.uv = [
                q.uv[0] + (k[0] - r[0]) * du,
                q.uv[1] + (k[1] - r[1]) * dv,
                q.uv[0] + (k[0] + k[2] - r[0]) * du,
                q.uv[1] + (k[1] + k[3] - r[1]) * dv,
            ];
            q.rect = k;
        }
    }
    out.push(q);
}

/// Which layouts to draw and how to animate triggered behaviours.
#[derive(Default)]
pub struct DrawOptions<'a> {
    /// Only the layouts (indices into [`UiScene::layouts`]) this returns true for.
    pub layouts: Option<Box<dyn Fn(usize) -> bool + 'a>>,
    /// Trigger state for timed tweens; `None` shows every triggered behaviour settled.
    pub anim: Option<&'a mut UiAnim>,
    /// Seconds, on any clock `anim` keeps seeing.
    pub now: f64,
    layout: u32,
    cell: u32,
    /// The current layout instance's `Binding_Replacement_Index` (`#` in image bindings).
    replacement: u32,
}

/// Animation state: per (element UniqueId, trigger index, layout index, stack cell) the trigger's value and
/// when it last changed; per (element, layout, cell) when the element appeared and when it was last drawn.
#[derive(Debug, Default, Clone)]
pub struct UiAnim {
    triggers: HashMap<(u32, u32, u32, u32), (bool, f64)>,
    shown: HashMap<(u32, u32, u32), (f64, f64)>,
}

/// An element not drawn for this long (s) plays its transition-in again when it is drawn.
const REAPPEAR_GAP: f64 = 0.25;

impl UiAnim {
    /// Forget every trigger (the next frame shows settled states).
    pub fn reset(&mut self) {
        self.triggers.clear();
        self.shown.clear();
    }
}

/// What the behaviours do to an element at one moment.
#[derive(Debug, Clone, Copy)]
struct Tween {
    opacity: f32,
    offset: [f32; 2],
    /// Degrees added by idle spins.
    rotation: f32,
    /// Size factor about the element's centre (behaviour kind 4).
    scale: f32,
    /// The element's own tint (linear RGBA), what colour steps start from and `OUT` returns to.
    base: [f32; 4],
    /// The tint now, when a colour step has changed it.
    colour: Option<[f32; 4]>,
}

/// Behaviour kinds (`Behaviour.Type`): 4 scales (`from,to` in percent), 13 changes the colour (`TO,<palette
/// entry>`, `TO,INVISIBLE`, `OUT` = back to the element's own colour). Our reading of the data (the tween code
/// was not traced): the search circle's `OUT` steps (kind 13) on its cooldown / race triggers would hide it in
/// every chase if they faded it, but the original shows it (captures 2026-10-19); the cop arrow's chase-ping
/// (`100,500`, kind 4) is a growing icon there, not an offset.
const KIND_SCALE: u8 = 4;
const KIND_COLOUR: u8 = 13;

impl Tween {
    fn new(base: [f32; 4]) -> Self {
        Self { opacity: 1.0, offset: [0.0; 2], rotation: 0.0, scale: 1.0, base, colour: None }
    }

    /// Apply behaviour steps `since` ms after they started. Each step runs from its `Delay` over its `Duration`
    /// (ms), in order: `IN` fades in from invisible (and holds the element invisible until it starts), `OUT`
    /// fades out, `TO,INVISIBLE` hides (over its duration), `dx,dy` approaches the rest position from that
    /// offset (reference pixels; sitting at the offset until it starts), as the layouts' `APPROACH (delay,
    /// duration, dx, dy)`; kind 4 `a,b` scales from a to b percent (at a until it starts); kind 13 `TO,<name>`
    /// blends the tint to the palette entry (`UIColourPalette`, [`UiScene::palette`]) and `OUT` back to the
    /// element's own tint. Bare ids (sounds) are not applied. The vocabulary is the data's; what each step does
    /// is our reading of the names (the tween code was not traced).
    /// [`apply`](Self::apply) for loops and idle lists: steps that have not started yet do nothing (an `IN`
    /// waiting for its delay does not hold the element invisible).
    fn apply_started(&mut self, steps: &[fmt::ui::Behaviour], since: f32, palette: &HashMap<String, [f32; 4]>) {
        for b in steps.iter().filter(|b| since >= b.delay) {
            self.apply(std::slice::from_ref(b), since, palette);
        }
    }

    fn apply(&mut self, steps: &[fmt::ui::Behaviour], since: f32, palette: &HashMap<String, [f32; 4]>) {
        let mut scaled = false;
        for b in steps {
            let started = since >= b.delay;
            let p = if b.duration <= 0.0 {
                if started { 1.0 } else { 0.0 }
            } else {
                ((since - b.delay) / b.duration).clamp(0.0, 1.0)
            };
            let params = b.parameters.trim();
            if params == "TO,INVISIBLE" {
                if started {
                    self.opacity *= 1.0 - p;
                }
                continue;
            }
            match b.kind {
                KIND_COLOUR => {
                    let target = match params {
                        "OUT" => Some(self.base),
                        _ => params.strip_prefix("TO,").and_then(|name| palette.get(name.trim()).copied()),
                    };
                    if let Some(target) = target
                        && started
                    {
                        let from = self.colour.unwrap_or(self.base);
                        self.colour = Some(std::array::from_fn(|i| from[i] + (target[i] - from[i]) * p));
                    }
                }
                KIND_SCALE => {
                    let parts: Vec<f32> = params.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if let [a, z] = parts[..]
                        && (started || !scaled)
                    {
                        self.scale = (a + (z - a) * p) / 100.0;
                        scaled |= started;
                    }
                }
                _ => match params {
                    "IN" => self.opacity = p,
                    "OUT" if started => self.opacity *= 1.0 - p,
                    _ => {
                        let parts: Vec<&str> = params.split(',').collect();
                        if let [x, y] = parts[..]
                            && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
                        {
                            self.offset[0] += x * (1.0 - p);
                            self.offset[1] += y * (1.0 - p);
                        }
                    }
                },
            }
        }
    }
}

impl<'a> DrawOptions<'a> {
    pub fn new(layouts: Option<Box<dyn Fn(usize) -> bool + 'a>>, anim: Option<&'a mut UiAnim>, now: f64) -> Self {
        Self { layouts, anim, now, layout: 0, cell: u32::MAX, replacement: 0 }
    }

    /// The element's behaviours now: its own transition-in since it appeared (was drawn after not being
    /// drawn), then each triggered behaviour (a `TriggerBinding` whose rising edge plays the transition-in
    /// steps and falling edge the transition-out steps). Without `anim`, and for a trigger that has not changed
    /// since it was first seen, the steps count as long finished. Transition-out of disappearing elements is
    /// not played. Order (our reading): the transition-in, the triggers that do not hold (their out steps), the
    /// element's idle loop, then the triggers that hold (their in steps and idle steps), so a held trigger's
    /// colour or hide wins over the idle loop and a released one's settled steps do not stop it (the minimap's
    /// pursuit flash 1444098 blinks on `TO,INVISIBLE` / `TO,HUD_PURSUIT_HEATFILL` while `Mode.IsRaceEvent` is
    /// false, whose out step is `OUT` to the own colour).
    fn tween(&mut self, e: &Element, lookup: &mut Lookup, palette: &HashMap<String, [f32; 4]>) -> Tween {
        let mut t = Tween::new(e.tint);
        // Idle behaviours of type 8 spin the element by `Parameters` degrees every `Duration` ms, repeating (the
        // loading spinner of layout 108177: "360.0" over 1000 ms). Our reading of the type; the other idle
        // types are not played. Only animated draws spin.
        if self.anim.is_some() {
            for b in e.idle.iter().filter(|b| b.kind == 8 && b.duration > 0.0) {
                if let Ok(deg) = b.parameters.trim().parse::<f32>() {
                    let ms = self.now * 1000.0 - b.delay as f64;
                    t.rotation += deg * (ms / b.duration as f64).rem_euclid(1.0) as f32;
                }
            }
        }
        let (now, layout, cell) = (self.now, self.layout, self.cell);
        if !e.transition_in.is_empty() {
            let since = match self.anim.as_deref_mut() {
                Some(anim) => {
                    let entry = anim.shown.entry((e.id, layout, cell)).or_insert((now, now));
                    if now - entry.1 > REAPPEAR_GAP {
                        entry.0 = now;
                    }
                    entry.1 = now;
                    ((now - entry.0) * 1000.0) as f32
                }
                None => f32::INFINITY,
            };
            t.apply(&e.transition_in, since, palette);
        }
        let mut held: Vec<(usize, f32)> = Vec::new();
        for (k, tb) in e.triggered.iter().enumerate() {
            if tb.trigger.is_empty() {
                continue;
            }
            let on = eval(&tb.trigger, lookup).is_some_and(|v| v.truthy());
            let since = match self.anim.as_deref_mut() {
                Some(anim) => {
                    let entry = anim.triggers.entry((e.id, k as u32, layout, cell)).or_insert((on, f64::NEG_INFINITY));
                    if entry.0 != on {
                        *entry = (on, now);
                    }
                    ((now - entry.1) * 1000.0) as f32
                }
                None => f32::INFINITY,
            };
            if on {
                held.push((k, since));
            } else {
                t.apply(&tb.transition_out, since, palette);
            }
        }
        // The element's own idle steps loop over their span (the prompt's pulse: `OUT` 0–0.8 s, `IN` 0.8–1.8 s;
        // the search circle's colour: `TO,COP` at 1 s, `TO,HUD_MINIMAP_PURSUITRADIUS` at 2 s), timed from when
        // the element appeared; only animated draws loop. After the first pass a loop starts from the colour its
        // last colour step left (the circle is red until it turns blue at 1 s).
        let looped = |b: &&fmt::ui::Behaviour| match b.kind {
            0 => matches!(b.parameters.trim(), "IN" | "OUT"),
            KIND_COLOUR | KIND_SCALE => true,
            _ => false,
        };
        let period = e.idle.iter().filter(looped).map(|b| b.delay + b.duration).fold(0.0, f32::max);
        if period > 0.0
            && let Some(anim) = self.anim.as_deref_mut()
        {
            let appeared = anim.shown.entry((e.id, layout, cell)).or_insert((now, now));
            if e.transition_in.is_empty() {
                if now - appeared.1 > REAPPEAR_GAP {
                    appeared.0 = now;
                }
                appeared.1 = now;
            }
            let elapsed = ((now - appeared.0) * 1000.0) as f32;
            let since = elapsed.rem_euclid(period);
            let steps: Vec<fmt::ui::Behaviour> = e.idle.iter().filter(looped).cloned().collect();
            if elapsed >= period
                && let Some(last) = steps.iter().rev().find(|b| b.kind == KIND_COLOUR)
            {
                t.apply(std::slice::from_ref(last), f32::INFINITY, palette);
            }
            t.apply_started(&steps, since, palette);
        }
        // A held trigger's in steps and idle steps run on top of the element's own idle: the title's prompt pair
        // (layout 1274392: label 1597779 "PRESS START" hides on `Controls.IsXInputControllerConnected(NOT)`,
        // label 1274432 "PRESS ANY KEY" on `Controls.IsXInputControllerConnected`) hide each other with an
        // instant `OUT`, which the looping pulse's `IN` must not undo (it did: both prompts showed over each
        // other). The order is our reading: the tween code was not traced.
        for (k, since) in held {
            t.apply(&e.triggered[k].transition_in, since, palette);
            t.apply_started(&e.triggered[k].idle, since, palette);
        }
        t
    }
}

/// A layout a `Sequence` shows (`LayoutSequenceItem`), from `start` to `end` seconds of the sequence's clock.
#[derive(Debug, Clone)]
pub struct SequenceLayout {
    pub start: f32,
    pub end: f32,
    /// `EnabledBinding` (empty = always).
    pub enabled: String,
    /// `Text1` (`bf1ce9a1`), `Text2`, `Text3`: language ids the layout reads as `UISequences.Params[#].TextN`.
    pub texts: [u32; 3],
    pub layout: PlacedLayout,
}

impl AssetStore {
    /// The `LayoutSequenceItem`s of a `Sequence` (by UniqueId), in item order; the other items (waves,
    /// PostFX, mixer channels) are left out.
    pub fn ui_sequence_layouts(&self, sequence: u32) -> Result<Vec<SequenceLayout>> {
        use fmt::genesys::{Value, field_hash as h};
        let o = self.genesys_object(fmt::ui::object_id(sequence))?;
        let num = |o: &fmt::genesys::Object, name: u32| match o.get(name) {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f32,
            Some(Value::UInt(u)) => *u as f32,
            _ => 0.0,
        };
        let handle = |o: &fmt::genesys::Object, name: &str| match o.get(h(name)) {
            Some(Value::Handle(Some(id))) | Some(Value::Ref(id)) => Some(*id),
            _ => None,
        };
        let mut out = Vec::new();
        let Some(Value::Array(items)) = o.get(h("SequenceItems")) else { return Ok(out) };
        for item in items {
            let Value::Struct(item) = item else { continue };
            if !item.type_name.ends_with("LayoutSequenceItem") {
                continue;
            }
            let Some(layout) = handle(item, "Layout").and_then(|l| self.ui_layout(l).ok()) else { continue };
            let instance = handle(item, "InstanceParams").and_then(|i| self.ui_layout_instance(i).ok());
            let enabled = match item.get(h("EnabledBinding")) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            };
            out.push(SequenceLayout {
                start: num(item, h("StartTime")),
                end: num(item, h("EndTime")),
                enabled,
                texts: [num(item, 0xbf1c_e9a1) as u32, num(item, h("Text2")) as u32, num(item, h("Text3")) as u32],
                layout: PlacedLayout {
                    widget_index: 0,
                    widget: String::new(),
                    widget_type: String::new(),
                    layout,
                    instance,
                },
            });
        }
        Ok(out)
    }
}

/// The minimap's source: a grid of map tiles (`HAWAII/MAPS/UIMAPGRIDS.BNDL`, `UIMapGrid`) covering the city.
#[derive(Debug, Clone)]
pub struct MapGrid {
    /// World x, z of the grid's corner.
    pub origin: [f32; 2],
    /// Tile edge (metres).
    pub tile_size: f32,
    pub columns: u32,
    pub rows: u32,
    /// Row-major (`row * columns + column`), texture resource ids.
    pub tiles: Vec<ResourceId>,
}

impl AssetStore {
    /// The largest map grid (8 × 10 tiles of 1036 m; the others are single tiles).
    pub fn ui_map_grid(&self) -> Result<MapGrid> {
        use fmt::genesys::{Value, field_hash as h};
        let b = self.bundle("HAWAII/MAPS/UIMAPGRIDS.BNDL").ok_or(Error::Missing(ResourceId::NULL))?;
        let mut best: Option<MapGrid> = None;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let o = self.decode_genesys(&b.load(e)?)?;
            if o.type_name != "Genesys.Gen.UIMapGrid" {
                continue;
            }
            let num = |name: &str| match o.get(h(name)) {
                Some(Value::Float(f)) => *f,
                Some(Value::Int(i)) => *i as f32,
                Some(Value::UInt(u)) => *u as f32,
                _ => 0.0,
            };
            let origin = match o.get(h("Position")) {
                Some(Value::Vector(v)) => [v[0], v[2]],
                _ => [0.0; 2],
            };
            let tiles: Vec<ResourceId> = match o.get(h("Textures")) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| match v {
                        Value::Int(i) => Some(fmt::ui::object_id(*i as u32)),
                        Value::UInt(u) => Some(fmt::ui::object_id(*u as u32)),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let grid = MapGrid {
                origin,
                tile_size: num("Size"),
                columns: num("Width") as u32,
                rows: num("Height") as u32,
                tiles,
            };
            if best.as_ref().is_none_or(|b| grid.tiles.len() > b.tiles.len()) {
                best = Some(grid);
            }
        }
        best.ok_or(Error::Missing(ResourceId::NULL))
    }
}

/// The map tiles stitched into one greyscale image.
#[derive(Debug, Clone)]
pub struct MapMosaic {
    pub grid: MapGrid,
    pub tile_px: u32,
    pub width: u32,
    pub height: u32,
    /// Luminance, row-major; image row 0 is the grid's first row.
    pub pixels: Vec<u8>,
}

impl MapMosaic {
    pub fn build(store: &AssetStore, grid: MapGrid) -> Self {
        let tile_px = 256u32;
        let (width, height) = (grid.columns * tile_px, grid.rows * tile_px);
        let mut pixels = vec![0u8; width as usize * height as usize];
        for (i, &id) in grid.tiles.iter().enumerate() {
            let (col, row) = (i as u32 % grid.columns.max(1), i as u32 / grid.columns.max(1));
            // Tiles past `columns × rows` have no place in the mosaic.
            if col >= grid.columns || row >= grid.rows {
                continue;
            }
            let Ok(t) = store.texture(id) else { continue };
            let Some((w, h, rgba)) = t.decode_rgba8(0) else { continue };
            for y in 0..tile_px {
                for x in 0..tile_px {
                    let (sx, sy) = (x * w / tile_px, y * h / tile_px);
                    let v = rgba[((sy * w + sx) * 4) as usize];
                    pixels[(row * tile_px + y) as usize * width as usize + (col * tile_px + x) as usize] = v;
                }
            }
        }
        Self { grid, tile_px, width, height, pixels }
    }

    /// Mosaic pixel coordinates of a world position.
    pub fn to_pixel(&self, x: f32, z: f32) -> [f32; 2] {
        let s = self.tile_px as f32 / self.grid.tile_size.max(1.0);
        [(x - self.grid.origin[0]) * s, (z - self.grid.origin[1]) * s]
    }

    /// Luminance (0–1) at a world position, bilinear; 0 outside the grid.
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let [px, py] = self.to_pixel(x, z);
        let get = |x: i64, y: i64| -> f32 {
            if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
                return 0.0;
            }
            self.pixels[(y as u64 * self.width as u64 + x as u64) as usize] as f32 / 255.0
        };
        let (ix, iy) = ((px - 0.5).floor() as i64, (py - 0.5).floor() as i64);
        let (tx, ty) = (px - 0.5 - ix as f32, py - 0.5 - iy as f32);
        let top = get(ix, iy) * (1.0 - tx) + get(ix + 1, iy) * tx;
        let bottom = get(ix, iy + 1) * (1.0 - tx) + get(ix + 1, iy + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// Render a heading-up view centred on world `(x, z)` into `out` (RGBA8, `w`×`h`, alpha 255) at
    /// `metres_per_px`. `forward` is the player's facing direction on the ground plane (x, z).
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self, x: f32, z: f32, forward: [f32; 2], metres_per_px: f32, w: u32, h: u32, out: &mut [u8]) {
        let len = (forward[0] * forward[0] + forward[1] * forward[1]).sqrt().max(1e-6);
        let (fx, fz) = (forward[0] / len, forward[1] / len);
        // Game coordinates are right-handed with Y up: seen from above, right of forward is (-fz, fx)…
        // or its mirror; MAP_RIGHT_SIGN picks the one that matches the tiles (research log).
        let (rx_dir, rz_dir) = (-fz * MAP_RIGHT_SIGN, fx * MAP_RIGHT_SIGN);
        for py in 0..h {
            for px in 0..w {
                let right = (px as f32 + 0.5 - w as f32 * 0.5) * metres_per_px;
                let up = (h as f32 * 0.5 - py as f32 - 0.5) * metres_per_px;
                let v = (self.sample(x + up * fx + right * rx_dir, z + up * fz + right * rz_dir) * 255.0) as u8;
                let i = ((py * w + px) * 4) as usize;
                if let Some(p) = out.get_mut(i..i + 4) {
                    p.copy_from_slice(&[v, v, v, 255]);
                }
            }
        }
    }
}

/// Which side of the forward direction is screen-right on the minimap (see [`MapMosaic::render`]).
pub const MAP_RIGHT_SIGN: f32 = 1.0;

#[cfg(test)]
mod tests {
    use super::UiFrame;

    #[test]
    fn hud_frame_follows_the_code() {
        // 16:9: the frame fills the screen; `Left` is the code's half pixel.
        let f = UiFrame::new([1920.0, 1080.0]);
        assert_eq!((f.scale, f.origin), (1.5, [0.0, 0.0]));
        assert!((f.left - 0.5).abs() < 1e-3 && (f.effective_width - 1280.0).abs() < 1e-2);
        // 4:3: height-fitted, centred, left-side HUD in by 160 (`FUN_0078faa0`).
        let f = UiFrame::new([1024.0, 768.0]);
        assert!((f.scale - 768.0 / 720.0).abs() < 1e-6 && (f.left - 160.5).abs() < 1e-3 && f.right == -f.left);
        assert!(f.to_screen([160.0, 0.0])[0].abs() < 1e-3);
        // Three monitors: the HUD stays in the centre 16:9.
        assert!((UiFrame::new([5760.0, 1080.0]).left - 0.5).abs() < 1e-3);
        // Portrait: width-fitted, `Top` moves top layouts up.
        let f = UiFrame::new([720.0, 1280.0]);
        assert!((f.scale - 0.5625).abs() < 1e-6 && f.top < -700.0 && f.left == 0.0);
    }

    /// More tiles than `columns × rows` (or a grid without columns) must not index past the mosaic.
    #[test]
    fn mosaic_ignores_tiles_outside_the_grid() {
        use super::{MapGrid, MapMosaic};
        use crate::test_store::{store, texture};
        use mw_bundle::ResourceId;
        let id = ResourceId(0x42);
        let s = store(&[("MAP.BNDL", vec![texture(id, 1, 1, vec![200, 0, 0, 255])])]);
        let grid = |columns, rows| MapGrid { origin: [0.0; 2], tile_size: 100.0, columns, rows, tiles: vec![id; 3] };
        let m = MapMosaic::build(&s, grid(1, 1));
        assert_eq!((m.width, m.height, m.pixels.len()), (256, 256, 256 * 256));
        assert_eq!(m.pixels[0], 200);
        assert!(MapMosaic::build(&s, grid(0, 2)).pixels.is_empty());
    }

    /// A HUD corner layout as the race HUD's are placed (anchor 8, T(1280, 720) + `Right(AXES:X)`), with a solid
    /// and a rotated element: at any 16:9 size the quads are the 1280×720 ones scaled (no pixel size is assumed
    /// anywhere between the layout data and the quads), and on a wider screen the corner stays at the screen's
    /// right edge (`FUN_0078faa0`). User report 2026-11-18 (HUD wrong at 1920×1080).
    #[test]
    fn hud_layout_is_resolution_independent() {
        use super::{Lookup, PlacedLayout, UiScene};
        use mw_formats::ui::{Bindings, Element, ElementKind, Layout, LayoutInstance, Transform};
        let element = |id: u32, position: [f32; 2], rotation: f32| Element {
            kind: ElementKind::Shape,
            type_name: String::new(),
            id,
            position,
            size: [200.0, 80.0],
            rotation,
            anchor: 8,
            bindings: Bindings::default(),
            texture: None,
            renderables: Vec::new(),
            material: None,
            image_source: 0,
            tint: [1.0; 4],
            colour_palette: 0,
            opacity: 1.0,
            flip: [false; 2],
            text: None,
            background: Some([1.0; 4]),
            border_width: 0,
            pop_mask: false,
            margins: [0.0; 4],
            min_size: [0.0; 2],
            sizes_cell: false,
            sub_image: None,
            stack: None,
            idle: Vec::new(),
            transition_in: Vec::new(),
            transition_out: Vec::new(),
            triggered: Vec::new(),
        };
        let translate = |value: [f32; 3], binding: &str| Transform { kind: 1, value, binding: binding.into() };
        let scene = UiScene {
            layouts: vec![PlacedLayout {
                widget_index: 0,
                widget: String::new(),
                widget_type: String::new(),
                layout: Layout {
                    id: 1,
                    elements: vec![element(1, [1270.0, 710.0], 0.0), element(2, [900.0, 600.0], 30.0)],
                },
                instance: Some(LayoutInstance {
                    id: 2,
                    anchor_type: 8,
                    transforms: vec![
                        translate([1280.0, 720.0, 0.0], ""),
                        translate([0.0; 3], "UIConfig.HudAspectCorrection.Right(AXES:X)"),
                    ],
                    timeline: String::new(),
                    visibility: String::new(),
                    colours: Vec::new(),
                    timelines: Vec::new(),
                    replacement_index: 0,
                }),
            }],
            ..Default::default()
        };
        let mut none = |_: &str, _: Option<&str>| None;
        let lookup: &mut Lookup = &mut none;
        let small = scene.draw([1280.0, 720.0], lookup);
        let large = scene.draw([1920.0, 1080.0], lookup);
        assert_eq!((small.len(), large.len()), (2, 2));
        for (a, b) in small.iter().zip(&large) {
            for k in 0..4 {
                assert!((a.rect[k] * 1.5 - b.rect[k]).abs() < 1e-2, "{:?} vs {:?}", a.rect, b.rect);
            }
            assert_eq!(a.rotation, b.rotation);
        }
        // The corner element's right / bottom edges 10 reference px in from the screen's corner.
        assert!((large[0].rect[0] + large[0].rect[2] - (1920.0 - 15.0 - 0.75)).abs() < 1e-2, "{:?}", large[0].rect);
        assert!((large[0].rect[1] + large[0].rect[3] - (1080.0 - 15.0)).abs() < 1e-2);
        // 21:9 at the same height: the frame keeps its height and the corner layout follows the right edge.
        let wide = scene.draw([2560.0, 1080.0], lookup);
        assert!((wide[0].rect[0] + wide[0].rect[2] - (2560.0 - 15.0 - 0.75)).abs() < 1e-2, "{:?}", wide[0].rect);
        assert!((wide[0].rect[2] - 300.0).abs() < 1e-3);
    }

    #[test]
    fn menu_frame_letterboxes() {
        let f = UiFrame::letterbox([1024.0, 768.0]);
        assert!((f.scale - 0.8).abs() < 1e-6 && f.origin[0] == 0.0 && (f.origin[1] - 96.0).abs() < 1e-3);
        assert_eq!(f.left, 0.0);
        let bars = f.bars([1024.0, 768.0]);
        assert_eq!(bars.len(), 2);
        assert!((bars[0].rect[3] - 96.0).abs() < 1e-3 && bars[0].rect[2] == 1024.0);
        assert!(UiFrame::letterbox([1280.0, 720.0]).bars([1280.0, 720.0]).is_empty());
    }
}
