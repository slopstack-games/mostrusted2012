//! The logic's load requests under `--hsm` (logic.rs): the loads the original's engine runs for its HSM, done here
//! on threads, and the load queries answered from them, so the logic waits where the original waits.
//!
//! - **Feedback message groups** (`LOAD_FDBK_GR` → `FUN_007982e0` on the UI manager +0x32858: count the group up
//!   when it holds a slot, else start loading it; `UNLOADONEXIT` → `FUN_00798370` counts it down):
//!   [`Request::LoadFeedbackGroup`] reads `EN_US/FEEDBACKGROUPS/<group>.BNDL` into the asset store and decodes its
//!   `FeedbackMessageGroup` (`AssetStore::feedback_group_sequences`); `Query::FeedbackGroupLoaded` (`FUN_00792590`:
//!   a slot with the id in state 3) is true once that read ended and while the count is above 0.
//! - **Prefetched screens** (`PREFETCHSCRE` → `FUN_007f0e10` / `FUN_007f0ef0` / `FUN_007f0e70`): the screen's
//!   bundle `UI/SCREENS2/<asset>.BNDL` is read into the asset store; `Query::ScreenPrefetched` once read.
//! - **UI layouts** (`HSB_RNLAYOUT` → `FUN_007a2a00`, `FUN_00791bd0`, `FUN_00791d60`, `FUN_00791a10`): the layout is
//!   decoded (`AssetStore::ui_layout`); `Query::LayoutReady` once decoded; released with the slot.
//!   The runtime does not draw these layouts (the only one on the boot path is 100143 "AlwaysAvailable
//!   Notifications").
//! - **The profile** (`LOADPROFILE` → `FUN_0071f010` / `FUN_0073daf0`): the save system is always ready
//!   (`Query::ProfileReady`); [`Request::LoadProfile`] reads our profile file (`frontend::load_profile`) and the
//!   settings it carries (`settings.cfg` through `settings::load`, with `--volume` on top) on a thread;
//!   `Query::ProfileLoaded` once read (a missing file is a new profile: loaded, as the original's fresh one);
//!   `ApplyProfileSettings` applies the read volumes to `settings::AudioSettings` (logic.rs).
//!
//! Model decisions: a group, screen or layout whose read fails counts as loaded (logged; the original would wait
//! forever on a missing resource); groups the logic never asked for keep the default answer (loaded): the
//! original's engine has some loaded before the HSM waits for them (97039 at "Wait For Required Resources", 97037,
//! 457707 at "Wait For Message Groups", 1560603 at "Wait for Recap FMG"; `UIConfig`'s
//! `LocalisedFeedbackMessageGroupList` names 457707 and 1560603) by a path not traced; nothing is evicted from the
//! asset store's cache when a count reaches 0 (the answers follow the count).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::*;
use mw_logic::hsm::behaviours::Query;
use mw_logic::natives::Request;

use crate::settings::AudioSettings;

/// A counted load (feedback groups, prefetched screens).
struct Counted {
    count: u32,
    loaded: bool,
    /// The frame the load started.
    since: u64,
}

/// A layout slot.
struct LayoutSlot {
    ready: bool,
    since: u64,
}

enum ProfileLoad {
    Idle,
    Loading { since: u64 },
    Loaded,
}

/// A finished load.
enum Done {
    Feedback(i64, Result<usize, String>),
    Screen(i64, bool),
    Layout(i64, Result<usize, String>),
    Profile(Option<crate::frontend::Profile>, AudioSettings),
}

/// The loads the logic asked for and their state (see the module doc).
pub struct Loads {
    store: Arc<mw_assets::AssetStore>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// Our profile file (`None`: `--new-game`, a brand new profile) and the `--volume` overrides.
    profile_path: Option<PathBuf>,
    volume: Option<String>,
    feedback: HashMap<i64, Counted>,
    screens: HashMap<i64, Counted>,
    layouts: HashMap<i64, LayoutSlot>,
    profile: ProfileLoad,
    /// The settings read with the profile (applied on `ApplyProfileSettings`).
    settings: Option<AudioSettings>,
}

impl Loads {
    pub fn new(store: Arc<mw_assets::AssetStore>, profile_path: Option<PathBuf>, volume: Option<String>) -> Self {
        let (tx, rx) = channel();
        Self {
            store,
            tx,
            rx,
            profile_path,
            volume,
            feedback: HashMap::new(),
            screens: HashMap::new(),
            layouts: HashMap::new(),
            profile: ProfileLoad::Idle,
            settings: None,
        }
    }

    fn spawn(&self, name: &str, job: impl FnOnce(&mw_assets::AssetStore) -> Done + Send + 'static) {
        let (tx, store) = (self.tx.clone(), self.store.clone());
        let spawned = crate::platform::thread::Builder::new().name(name.into()).spawn(move || {
            let _ = tx.send(job(&store));
        });
        if let Err(e) = spawned {
            warn!("hsm: {name}: no loader thread: {e}");
        }
    }

    /// Carry out a load request; false when it is not one.
    pub fn request(&mut self, r: &Request, frame: u64) -> bool {
        match r {
            Request::LoadFeedbackGroup { group } => {
                let group = *group;
                let e = self.feedback.entry(group).or_insert(Counted { count: 0, loaded: false, since: frame });
                e.count += 1;
                if e.count == 1 && !e.loaded {
                    e.since = frame;
                    self.spawn("mw-feedback", move |store| {
                        let id = u32::try_from(group).unwrap_or(0);
                        Done::Feedback(
                            group,
                            store.feedback_group_sequences(id).map(|s| s.len()).map_err(|e| e.to_string()),
                        )
                    });
                }
            }
            Request::UnloadFeedbackGroup { group } => {
                if let Some(e) = self.feedback.get_mut(group) {
                    e.count = e.count.saturating_sub(1);
                    if e.count == 0 {
                        e.loaded = false;
                        info!("hsm: {frame:5} feedback group {group} released");
                    }
                }
            }
            Request::PrefetchScreen { asset } => {
                let asset = *asset;
                let e = self.screens.entry(asset).or_insert(Counted { count: 0, loaded: false, since: frame });
                e.count += 1;
                if e.count == 1 && !e.loaded {
                    e.since = frame;
                    self.spawn("mw-prefetch", move |store| {
                        let id = u32::try_from(asset).unwrap_or(0);
                        Done::Screen(asset, store.bundle(mw_assets::AssetStore::ui_screen_bundle(id)).is_some())
                    });
                }
            }
            Request::ReleasePrefetchedScreen { asset } => {
                if let Some(e) = self.screens.get_mut(asset) {
                    e.count = e.count.saturating_sub(1);
                    if e.count == 0 {
                        self.screens.remove(asset);
                    }
                }
            }
            Request::RunLayout { layout, .. } => {
                let layout = *layout;
                self.layouts.insert(layout, LayoutSlot { ready: false, since: frame });
                self.spawn("mw-layout", move |store| {
                    let id = mw_formats::ui::object_id(u32::try_from(layout).unwrap_or(0));
                    Done::Layout(layout, store.ui_layout(id).map(|l| l.elements.len()).map_err(|e| e.to_string()))
                });
            }
            // Nothing drawn, nothing to hide.
            Request::DeactivateLayout { .. } => {}
            Request::ReleaseLayout { layout } => {
                self.layouts.remove(layout);
            }
            Request::LoadProfile => {
                self.profile = ProfileLoad::Loading { since: frame };
                let (path, volume) = (self.profile_path.clone(), self.volume.clone());
                self.spawn("mw-profile", move |_| {
                    let profile = path.as_deref().and_then(crate::frontend::load_profile);
                    Done::Profile(profile, crate::settings::load(volume.as_deref()))
                });
            }
            _ => return false,
        }
        true
    }

    /// Hand over the finished loads (logged with the frames they took).
    pub fn poll(&mut self, frame: u64) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Feedback(group, got) => {
                    let Some(e) = self.feedback.get_mut(&group) else { continue };
                    match got {
                        Ok(n) => info!(
                            "hsm: {frame:5} feedback group {group} loaded ({n} sequences, {} frames)",
                            frame - e.since
                        ),
                        Err(err) => warn!("hsm: {frame:5} feedback group {group}: {err} (counts as loaded)"),
                    }
                    e.loaded = e.count > 0;
                }
                Done::Screen(asset, found) => {
                    let Some(e) = self.screens.get_mut(&asset) else { continue };
                    if found {
                        info!("hsm: {frame:5} screen {asset} prefetched ({} frames)", frame - e.since);
                    } else {
                        warn!("hsm: {frame:5} screen {asset}: no bundle (counts as prefetched)");
                    }
                    e.loaded = true;
                }
                Done::Layout(layout, got) => {
                    let Some(s) = self.layouts.get_mut(&layout) else { continue };
                    match got {
                        Ok(n) => info!(
                            "hsm: {frame:5} layout {layout} ready ({n} elements, {} frames; not drawn)",
                            frame - s.since
                        ),
                        Err(err) => warn!("hsm: {frame:5} layout {layout}: {err} (counts as ready)"),
                    }
                    s.ready = true;
                }
                Done::Profile(profile, settings) => {
                    let since = match self.profile {
                        ProfileLoad::Loading { since } => since,
                        _ => frame,
                    };
                    match &profile {
                        Some(p) => info!("hsm: {frame:5} profile loaded ({}, {} frames)", p.bundle, frame - since),
                        None => {
                            info!("hsm: {frame:5} profile loaded (none saved: a new profile, {} frames)", frame - since)
                        }
                    }
                    self.profile = ProfileLoad::Loaded;
                    self.settings = Some(settings);
                }
            }
        }
    }

    /// The settings read with the profile (`ApplyProfileSettings`).
    pub fn settings(&self) -> Option<AudioSettings> {
        self.settings.clone()
    }

    /// The load queries' answers; `None` for the others and for loads the logic never asked for.
    pub fn query(&self, q: &Query) -> Option<u64> {
        Some(match q {
            // Our save system takes a load at any time (G+0x2f454 +0x4934).
            Query::ProfileReady => !matches!(self.profile, ProfileLoad::Loading { .. }) as u64,
            Query::ProfileLoaded => match self.profile {
                ProfileLoad::Idle => return None,
                ProfileLoad::Loading { .. } => 0,
                ProfileLoad::Loaded => 1,
            },
            Query::FeedbackGroupLoaded { group } => self.feedback.get(group)?.loaded as u64,
            Query::ScreenPrefetched { asset } => self.screens.get(asset)?.loaded as u64,
            Query::LayoutReady { layout } => self.layouts.get(layout)?.ready as u64,
            _ => return None,
        })
    }
}
