use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

mod camera_take;
mod compare;
mod crack;
mod damage;
mod events;
#[cfg(target_os = "linux")]
mod memscan;
mod placement;
mod preview;
mod props;
mod survey;
#[cfg(target_os = "linux")]
mod traffic_sound;
mod tune;
mod ui;
mod verify_handling;

use mw_assets::AssetStore;
use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats as fmt;
use mw_gamedir::GameDir;
use rayon::prelude::*;

#[derive(Parser)]
#[command(version, about = "Inspect and extract NFS Most Wanted (2012) assets")]
struct Cli {
    /// Game install directory (defaults to $MW2012_DIR or Steam auto-detection).
    #[arg(long, global = true)]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum UiCmd {
    /// One line per screen bundle with its widgets (and their layout counts).
    Screens,
    /// The Easydrive menu model of a driving screen: item types, tiers, sources and items.
    Easydrive {
        #[arg(default_value = mw_assets::easydrive::EASYDRIVE_SCREEN)]
        screen: String,
        #[arg(long, default_value = mw_assets::DEFAULT_LANGUAGE)]
        lang: String,
    },
    /// Print a screen's widgets, layouts and elements.
    Show {
        /// Screen bundle, e.g. `UI/SCREENS2/371670.BNDL` (the free-roam HUD).
        screen: String,
        /// Only these widgets (comma-separated names).
        #[arg(long, value_delimiter = ',')]
        widget: Vec<String>,
        #[arg(long, default_value = mw_assets::DEFAULT_LANGUAGE)]
        lang: String,
    },
    /// Draw a screen's layouts to a PNG.
    Render {
        screen: String,
        out: PathBuf,
        #[arg(long, value_delimiter = ',')]
        widget: Vec<String>,
        #[arg(long, default_value = mw_assets::DEFAULT_LANGUAGE)]
        lang: String,
        /// Output scale relative to 1280x720.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Only these layouts (UniqueIds, comma-separated).
        #[arg(long, value_delimiter = ',')]
        layouts: Vec<u32>,
        /// Binding values, `path=value` (e.g. `Players.GetLocalCompetitor.GetPlayerSpeed=88`,
        /// `Language.IsMetric=1`); repeatable.
        #[arg(long = "set")]
        values: Vec<String>,
        /// Show every element: unknown bindings count as 1 instead of hiding their elements.
        #[arg(long)]
        all: bool,
        /// Draw the minimap at world `x,z,heading_degrees`.
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
        map: Vec<f32>,
    },
    /// Print language strings by id, or every string containing `--grep` (case-insensitive).
    Text {
        ids: Vec<u32>,
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value = mw_assets::DEFAULT_LANGUAGE)]
        lang: String,
    },
    /// Draw a text sample in a UI font.
    Font {
        number: u32,
        out: PathBuf,
        #[arg(long, default_value = "Most Wanted 0123456789 km/h MPH\nÀÉÎÕÜ ÇÑ ßæø Ünïcödé ŁŚŻ")]
        text: String,
        #[arg(long, default_value_t = 40.0)]
        size: f32,
    },
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the detected game installation.
    Info,
    /// List resources in a bundle (path may be relative to the game dir).
    List { bundle: String },
    /// Dump every resource chunk of a bundle to a directory.
    Extract {
        bundle: String,
        out: PathBuf,
        /// Only extract this resource type (e.g. `Texture`).
        #[arg(long)]
        r#type: Option<String>,
    },
    /// Decompress every resource in every bundle and report statistics.
    Verify,
    /// Find which bundles contain a resource ID.
    Find { id: String },
    /// Export one texture (by resource ID) as .png.
    Texture { id: String, out: PathBuf },
    /// Export textures from a bundle as .dds (and optionally .png).
    Textures {
        bundle: String,
        out: PathBuf,
        #[arg(long)]
        png: bool,
    },
    /// Export the LOD0 geometry of every Model in a bundle as .obj and a
    /// software-rendered preview .png.
    Models {
        bundle: String,
        out: PathBuf,
        /// Print per-mesh shader / layout details.
        #[arg(long)]
        verbose: bool,
    },
    /// Dump GeneSys objects (a bundle's, or one by ID); fields show their name when known, else the hash.
    Genesys {
        /// Bundle path, GenesysObject resource ID, or `all` (scan every bundle; use with --type).
        target: String,
        /// Only objects whose type name contains this.
        #[arg(long)]
        r#type: Option<String>,
        /// Maximum array elements to print.
        #[arg(long, default_value_t = 16)]
        max_items: usize,
    },
    /// Every GenesysType in the install as TSV: type id, type name, kind, size, then one row per field
    /// (field index, name hash, offset, size, count, flags, field type name).
    GenesysTypes,
    /// Search word lists for GeneSys field names (see crates/mwtool/src/crack.rs). Prints
    /// `hash, candidates, types`; check hits against values before trusting them.
    GenesysCrack {
        /// Word list for each word position (the last one repeats), one word per line.
        #[arg(long = "words", required = true)]
        words: Vec<PathBuf>,
        #[arg(long, default_value_t = 3)]
        max_words: usize,
        #[arg(long = "style", value_enum, default_values_t = [crack::Style::Snake, crack::Style::TitleSnake, crack::Style::TitleSmall, crack::Style::Camel])]
        styles: Vec<crack::Style>,
        /// Only fields of types whose name contains this.
        #[arg(long)]
        r#type: Option<String>,
        /// Also search fields that already have a name.
        #[arg(long)]
        include_named: bool,
    },
    /// Read-only value scanner for the running original game (Linux / Proton): find the memory holding
    /// speed, drift state, ... for handling measurements. Interactive; `help` lists the commands. See
    /// crates/mwtool/src/memscan.rs and docs/handling.md ("Measuring the original").
    Memscan {
        /// Process to attach to (basename on its command line).
        #[arg(long, default_value = "NFS13.exe")]
        process: String,
        /// Attach to this pid instead.
        #[arg(long)]
        pid: Option<u32>,
        /// Watch file that `save` appends to.
        #[arg(long, default_value = "handling.watch")]
        watch: PathBuf,
        /// Also scan regions above 4 GiB (host libraries; the 32-bit game lives below).
        #[arg(long)]
        all_regions: bool,
    },
    /// Live tuning menu for the running original: handling constants and the player car's fields by page,
    /// changed with a keypress, restored on quit (data pokes only; docs/probe.md "Tuning menu").
    Tune {
        #[arg(long, default_value = "NFS13.exe")]
        process: String,
        #[arg(long)]
        pid: Option<u32>,
        /// Preset file for `s` (save) / `L` (load): `key = value [hold]` lines.
        #[arg(long, default_value = "ref/tune/preset.txt")]
        preset: PathBuf,
        /// Apply the preset at start.
        #[arg(long)]
        apply: bool,
        /// Print every field once and exit (no terminal UI).
        #[arg(long)]
        print: bool,
    },
    /// Sample the values of a watch file from the running game into CSV.
    Telemetry {
        watch: PathBuf,
        #[arg(long, short)]
        out: PathBuf,
        #[arg(long, default_value_t = 120.0)]
        hz: f64,
        /// Stop after this many seconds (default: until Ctrl-C or the game exits).
        #[arg(long)]
        seconds: Option<f64>,
        #[arg(long, default_value = "NFS13.exe")]
        process: String,
        #[arg(long)]
        pid: Option<u32>,
        /// Write a data value for the run and restore it afterwards: `watch-name=value` or
        /// `NFS13.exe+0x787340=1.2` (f32; `:i32` / `:u8` after the address for other types). Repeatable.
        /// Data mappings only; docs/probe.md "Tier 0".
        #[arg(long = "poke")]
        pokes: Vec<String>,
    },
    /// Record a reference run of the running original into `ref/<name>/`: its car state from the watch file
    /// (`telemetry.csv`) and, on PipeWire / PulseAudio, its own audio stream (`audio.wav`). Press Enter to stop.
    /// Replay it in Most Rusted with `mostrusted --drive <car> --replay ref/<name>/telemetry.csv` and compare
    /// with `mwtool compare`.
    Capture {
        name: String,
        #[arg(long, default_value = "handling.watch")]
        watch: PathBuf,
        #[arg(long, default_value_t = 60.0)]
        hz: f64,
        /// Stop after this many seconds (default: Enter, or the game exits).
        #[arg(long)]
        seconds: Option<f64>,
        /// Skip the audio recording.
        #[arg(long)]
        no_audio: bool,
        /// Write a data value for the run and restore it afterwards (see `telemetry --poke`).
        #[arg(long = "poke")]
        pokes: Vec<String>,
        #[arg(long, default_value = "NFS13.exe")]
        process: String,
        #[arg(long)]
        pid: Option<u32>,
    },
    /// Compare a recording of the original (`capture`) with Most Rusted's replay of it (`--replay`): speed, yaw
    /// rate, lateral g, body slip, rpm, gear, drift state and path side by side, and RMS differences.
    Compare {
        original: PathBuf,
        ours: PathBuf,
        /// Table step (s).
        #[arg(long, default_value_t = 0.5)]
        every: f32,
    },
    /// Check a ported handling function against the original, offline: reads a CSV of the game's per-wheel
    /// tyre inputs and its resulting coefficients (from `telemetry` / `capture` with per-wheel watches), runs
    /// `mw_handling`'s port on the same inputs and reports the error. No hooks (docs/probe.md, "Tier 2").
    VerifyHandling {
        /// CSV with per-wheel columns `<w>_kappa, <w>_alpha, [<w>_camber], <w>_mu_lat, <w>_mu_long`.
        csv: PathBuf,
        /// Car name or `.BNDL` path, to load its tyre data.
        #[arg(long)]
        vehicle: String,
        /// Wheel column prefixes (default `fl,fr,rl,rr`; those starting `f` are the front axle).
        #[arg(long)]
        wheels: Option<String>,
    },
    /// Decode game audio to .wav: an .SPS file, or a bundle's Ginsu/Wave resources.
    Audio {
        /// `.SPS` path (relative to the game dir) or bundle path.
        source: String,
        out: PathBuf,
    },
    /// Re-synthesise one Ginsu recording of a bundle through the granular synth: driven by the recording's
    /// own RPM curve (the output should match the recording), or held at a fixed RPM.
    GinsuResynth {
        bundle: String,
        /// Index of the Ginsu resource in the bundle (bundle order, as `audio` lists them).
        index: usize,
        out: PathBuf,
        /// Hold this RPM instead of following the recording.
        #[arg(long)]
        hold: Option<f32>,
        /// Seconds to render when holding (default 3).
        #[arg(long, default_value_t = 3.0)]
        seconds: f32,
    },
    /// Render a simulated rev-up/coast through the engine-sound synth to a .wav.
    EngineSound {
        car: String,
        out: PathBuf,
        /// Drive the synth from a recorded trace (`MW_ENGINE_TRACE`) instead of a sweep: lines of
        /// `seconds rpm throttle [gear [speed m/s]]`.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Render the older six-layer Ginsu mixer instead of the full sequence bank.
        #[arg(long)]
        legacy: bool,
        /// Bypass the mixer plug-ins (EQ, distortion, sub-harmoniser).
        #[arg(long)]
        dry: bool,
        /// Render the car's AI-racer engine (`<id>_COMPETITORSOUND`) instead of the player's.
        #[arg(long)]
        competitor: bool,
    },
    /// Traffic engine sounds (`<component>_COMPETITORSOUND` in `VEHICLES/TRAFFICATTRIBS.BNDL`,
    /// `mw_assets::car_sound`): lists each vehicle type's bank, its RPM against normalised speed and its 3D
    /// placement; with an output file, renders a 0 → top speed → 0 sweep (14 s) of one type and then a
    /// pass-by (`--passby mph`) through the game's falloff and doppler, printing the gain against distance.
    TrafficSound {
        /// A `VehicleComponent` id (e.g. 122816), or `all`.
        component: String,
        out: Option<PathBuf>,
        /// Pass-by speed (mph) for the 3D render: the car drives past a listener 4 m to the side.
        #[arg(long, default_value_t = 40.0)]
        passby: f32,
    },
    /// Render a scripted drive (skids, wheelspin, locked brakes, impacts, a scrape, wind) through the
    /// vehicle sound bank (tyre / road / wind / collision sounds) to a .wav.
    VehicleSound {
        out: PathBuf,
        #[arg(long)]
        dry: bool,
        /// Instead of the scripted drive: hold `--mph` and sweep one skid intensity binding (`lat`, `spin` or
        /// `braking`) from 0 to 1 over 10 s (after 1 s at 0, then 2 s at 1), and print the skid sequence's
        /// wave layers' gains (fade envelope × item gain) against the intensity.
        #[arg(long)]
        sweep: Option<String>,
        #[arg(long, default_value_t = 40.0)]
        mph: f32,
    },
    /// Count world triangles per shader (all track units), to prioritise ports.
    ShaderUsage,
    /// Sampler states (filter, anisotropy, LOD bias, address modes) per shader and sampler slot over the
    /// world meshes of all track units; `--filter` keeps shaders whose name contains it. `--textures` lists
    /// the bound textures' format, size and mip count instead (mips must reach 1x1 to filter at a distance).
    SamplerUsage {
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        textures: bool,
    },
    /// List every shader with its sampler names.
    Shaders,
    /// List the cars from the vehicle list with names and bundles.
    Vehicles,
    /// Show a car's damage data (skeleton, per-zone deformation clips, hinged parts), check its
    /// skinning, and optionally render a damaged pose (`--level front=0.5 --preview out.png`).
    /// `damage all` surveys every car.
    Damage {
        /// Car name, vehicle ID, bundle path, or `all`.
        car: String,
        /// Damage per zone as `zone=level` (zone name such as `front` / `rear-left`, or clip index;
        /// `all=level` sets every zone), level 0..1.
        #[arg(long)]
        level: Vec<String>,
        /// Write a software-rendered preview of the posed body.
        #[arg(long)]
        preview: Option<PathBuf>,
    },
    /// Show the handling data extracted from a car's GeneSys objects.
    Handling {
        /// Car name, vehicle ID or bundle path.
        car: String,
    },
    /// A car's performance mods (`Vehicles.PerformanceUpgrades`): slot -> choice -> parameter changes.
    Mods {
        /// Car name, vehicle ID or bundle path.
        car: String,
    },
    /// The collision surface list (`HAWAII/SURFACELIST.BNDL`) with each surface's triangle count and
    /// collision flags over the track units.
    Surfaces,
    /// ICE camera takes (`CAMERAS.BNDL`, `mw_formats::camera_take`): one line each, or one take (name,
    /// UniqueId or resource id) with its channel layout and the camera evaluated every `--step` seconds.
    CameraTake {
        take: Option<String>,
        #[arg(long, default_value_t = 0.25)]
        step: f32,
    },
    /// Free-roam objects from the world's TriggerData (`HAWAII/TRIGGERS.BNDL`): speed cameras, billboards,
    /// security gates, jack spots (location + car), body shops; counts per kind, then one line each with
    /// `--list` (or `--kind SpeedCam` etc.). `--all-kinds` adds the counts of every trigger kind byte.
    Freeroam {
        #[arg(long)]
        list: bool,
        /// Only this kind (telemetry name: SpeedCam, Billboard, SecGate, JackspotLocation, JackspotCar, BodyShop).
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        all_kinds: bool,
    },
    /// Drop every placement (instances, props, compounds, dynamics, groundcover) onto the track units'
    /// collision and report per layer / model / unit how far it stands off the ground.
    PlacementCheck {
        /// Track unit bundles to check (default: all; collision is always loaded from all).
        #[arg(long = "bundle")]
        bundles: Vec<String>,
        /// Rows per outlier list.
        #[arg(long, default_value_t = 25)]
        top: usize,
        /// Groundcover only: per group, which groups stand on the ground (by layer, blob, start, size exponent).
        #[arg(long)]
        groups: bool,
        /// Groundcover only: the sprites a camera at `eye x,y,z,target x,y,z` sees within 30° of its
        /// axis, with their distance, the ground under them and the game's per-sprite range.
        #[arg(long, allow_hyphen_values = true)]
        view: Option<String>,
        /// Every placed render mesh (main and backdrop) that covers the point `x,z`: unit, model, shader and
        /// the surface height there.
        #[arg(long, allow_hyphen_values = true)]
        column: Option<String>,
        /// Groundcover only: per group, whether the zone that owns its place lists its collection.
        #[arg(long)]
        owners: bool,
    },
    /// Physics of the world's props, compound and dynamic objects (`PhysicalDefinition`): volume kinds,
    /// masses and break forces over the track units (or the given bundles).
    Props {
        bundles: Vec<String>,
        /// One line per object definition.
        #[arg(long, short)]
        verbose: bool,
    },
    /// The game's UI (GeneSys screens and layouts): `ui screens` lists every screen's widgets, `ui show <screen>`
    /// prints layouts and elements, `ui render <screen> out.png` draws them at 1280x720 (every element, bindings
    /// ignored), `ui font <n> out.png` draws a sample in `UI/FONTS/<n>.BNDL`.
    Ui {
        #[command(subcommand)]
        cmd: UiCmd,
    },
    /// Disassemble the compiled D3D11 programs (DXBC) a shader uses.
    ShaderAsm {
        /// Exact shader name, e.g. `DriveableSurface`. With `--exe`: `list`, `all`, a program's address
        /// (`0x01016148`) or a word its disassembly contains (`Sampler3dTint`).
        name: String,
        /// The programs built into `NFS13.exe` (`.rdata`, not encrypted) instead of `SHADERS.BNDL`: the
        /// final post-process pass, the UI and sky helpers live there.
        #[arg(long)]
        exe: bool,
    },
    /// Per-field statistics of a resource type's header across the install.
    Survey {
        /// Resource type name, e.g. `VehicleGraphicsSpec`.
        r#type: String,
        /// Number of bytes to report.
        #[arg(long, default_value = "0x80", value_parser = parse_int)]
        len: usize,
        /// Maximum number of instances to sample.
        #[arg(long, default_value_t = 20000)]
        limit: usize,
        /// Survey chunk 1 (body) instead of chunk 0.
        #[arg(long)]
        body: bool,
    },
    /// Export the collision geometry (PolygonSoupList) of a bundle as .obj + preview.
    Collision { bundle: String, out: PathBuf },
    /// Traffic lane network (TrafficLaneData in HAWAII/TRAFFIC.BNDL): counts, flows, flow types, vehicles;
    /// `--png` draws the lane rungs top-down (colour = flow index, red = stop line section).
    Traffic {
        #[arg(long)]
        png: Option<PathBuf>,
        /// List the lane sections within 80 m of `x,z` (start, end, length, speed, density, links).
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
        at: Vec<f32>,
    },
    /// Offline events (`OfflineEvent` in GAMELOGIC/GAMEMODES.BNDL, resolved against HAWAII/TRIGGERS.BNDL): one
    /// line per event (id, kind, laps, checkpoints, opponents, name, car); `events <id>` prints one in detail,
    /// `--png` draws its route over the lane map (yellow route, white gates, green start, red opponents).
    Events {
        id: Option<String>,
        #[arg(long)]
        png: Option<PathBuf>,
        /// Also list events without checkpoints (free-roam placeholders, DLC modes).
        #[arg(long)]
        all: bool,
    },
    /// Show a WorldPaintMap (what is painted where: environment timelines, layer fog, particle lighting),
    /// which entries each track unit's zone covers, and optionally render it with the zone outlines.
    PaintMap {
        /// Resource ID of the map (010000000002823d = environment timelines, 01000000000285cb = layer fog).
        id: String,
        #[arg(long)]
        png: Option<PathBuf>,
        /// Print the entry painted at these `x,z` positions.
        #[arg(long = "at", value_delimiter = ',', allow_hyphen_values = true)]
        at: Vec<f32>,
    },
    /// Export placed world geometry (all TRK_UNIT bundles by default).
    World {
        out: PathBuf,
        /// Specific bundles, e.g. HAWAII/TRK_UNIT45.BNDL.
        #[arg(long = "bundle")]
        bundles: Vec<String>,
        /// Preview image size per view.
        #[arg(long, default_value_t = 1024)]
        size: u32,
        /// Also write scene.obj (can be several GB for the whole map).
        #[arg(long)]
        obj: bool,
        /// Which InstanceList entries to include: all, main, backdrop.
        #[arg(long, default_value = "all")]
        instances: String,
    },
    /// Decode a particle effect (`BearEffect`) by resource ID: parameters, layers, channels, actions
    /// and action groups. `all` prints a one-line summary of every effect.
    Effect {
        id: String,
        /// Also print every action's raw payload words.
        #[arg(long)]
        raw: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // The live-process commands don't need the install.
    match &cli.cmd {
        Cmd::Memscan { process, pid, watch, all_regions } => return memscan_cmd(process, *pid, watch, *all_regions),
        Cmd::Tune { process, pid, preset, apply, print } => return tune_cmd(process, *pid, preset, *apply, *print),
        Cmd::Telemetry { watch, out, hz, seconds, process, pid, pokes } => {
            return telemetry_cmd(watch, out, *hz, *seconds, process, *pid, pokes);
        }
        Cmd::Capture { name, watch, hz, seconds, no_audio, process, pid, pokes } => {
            return capture_cmd(name, watch, *hz, *seconds, *no_audio, process, *pid, pokes);
        }
        Cmd::Compare { original, ours, every } => return compare::compare(original, ours, *every),
        _ => {}
    }
    let game = GameDir::locate(cli.game_dir.as_deref())?;
    match cli.cmd {
        Cmd::Memscan { .. } | Cmd::Tune { .. } | Cmd::Telemetry { .. } | Cmd::Capture { .. } | Cmd::Compare { .. } => {
            unreachable!("handled before locating the game")
        }
        Cmd::Info => {
            println!("root:    {}", game.root().display());
            println!("version: {}", game.version().unwrap_or("unknown"));
            println!("bundles: {}", game.files_with_extension("BNDL").len());
        }
        Cmd::List { bundle } => list(&open_bundle(&game, &bundle)?),
        Cmd::Extract { bundle, out, r#type } => extract(&open_bundle(&game, &bundle)?, &out, r#type.as_deref())?,
        Cmd::Verify => verify(&game)?,
        Cmd::Effect { id, raw } => effect(&AssetStore::new(game)?, &id, raw)?,
        Cmd::Find { id } => {
            let id = ResourceId::parse(&id).context("invalid resource id")?;
            for rel in game.files_with_extension("BNDL") {
                let b = Bundle::open(game.root().join(&rel))?;
                if let Some(e) = b.find(id) {
                    println!("{}: {}", rel.display(), e.resource_type());
                }
            }
        }
        Cmd::Texture { id, out } => {
            let store = AssetStore::new(game)?;
            let tex = store.texture(ResourceId::parse(&id).context("invalid resource id")?)?;
            let (w, h, px) = tex.decode_rgba8(0).with_context(|| format!("cannot decode {:?}", tex.format))?;
            image::RgbaImage::from_raw(w, h, px).context("bad image size")?.save(&out)?;
            println!("{}x{} {:?} -> {}", tex.width, tex.height, tex.format, out.display());
        }
        Cmd::Textures { bundle, out, png } => export_textures(&open_bundle(&game, &bundle)?, &out, png)?,
        Cmd::Models { bundle, out, verbose } => {
            let b = open_bundle(&game, &bundle)?;
            export_models(&AssetStore::new(game)?, &b, &out, verbose)?
        }
        Cmd::Survey { r#type, len, limit, body } => {
            let ty = ResourceType::from_name(&r#type).with_context(|| format!("unknown type {type}", type = r#type))?;
            survey::run(&game, &survey::Options { ty, len, limit, body })?
        }
        Cmd::Ui { cmd } => {
            let store = AssetStore::new(game)?;
            fn names(w: &[String]) -> Vec<&str> {
                w.iter().map(String::as_str).collect()
            }
            match cmd {
                UiCmd::Screens => ui::screens(&store)?,
                UiCmd::Easydrive { screen, lang } => ui::easydrive(&store, &screen, &store.language(&lang)?)?,
                UiCmd::Show { screen, widget, lang } => {
                    let lang = store.language(&lang)?;
                    let w = names(&widget);
                    ui::show(&store, &screen, (!w.is_empty()).then_some(&w[..]), &lang)?
                }
                UiCmd::Render { screen, out, widget, lang, scale, layouts, values, all, map } => {
                    let lang = store.language(&lang)?;
                    let w = names(&widget);
                    let options = ui::RenderOptions {
                        widgets: (!w.is_empty()).then_some(&w[..]),
                        layouts: &layouts,
                        scale,
                        values: &values,
                        all,
                        map: (map.len() == 3).then(|| [map[0], map[1], map[2]]),
                    };
                    ui::render(&store, &screen, &lang, &out, &options)?
                }
                UiCmd::Text { ids, grep, lang } => {
                    let lang = store.language(&lang)?;
                    for id in &ids {
                        println!("{id}\t{:?}", lang.get(*id));
                    }
                    if let Some(g) = grep.map(|g| g.to_lowercase()) {
                        let mut hits: Vec<_> =
                            lang.strings.iter().filter(|(_, s)| s.to_lowercase().contains(&g)).collect();
                        hits.sort();
                        for (id, s) in hits {
                            println!("{id}\t{s:?}");
                        }
                    }
                }
                UiCmd::Font { number, out, text, size } => {
                    ui::font_sample(&store, number, &text.replace("\\n", "\n"), size, &out)?
                }
            }
        }
        Cmd::ShaderAsm { name, exe: true } => exe_shader_asm(&game, &name)?,
        Cmd::ShaderAsm { name, exe: false } => {
            let store = AssetStore::new(game)?;
            let b = store.bundle("SHADERS.BNDL").context("SHADERS.BNDL missing")?;
            let entry = b
                .entries_of_type(ResourceType::Shader)
                .find(|e| b.load(e).ok().and_then(|r| fmt::Shader::parse(&r).ok()).is_some_and(|s| s.name == name))
                .with_context(|| format!("no shader named {name}"))?;
            let shader = b.load(entry)?;
            // Pixel programs are imported directly; vertex programs sit behind
            // VertexProgramState resources.
            let mut seen = std::collections::HashSet::new();
            let mut queue: Vec<(String, mw_bundle::Import)> =
                shader.imports.iter().map(|i| ("shader".to_string(), *i)).collect();
            while let Some((via, imp)) = queue.pop() {
                if !seen.insert(imp.id) {
                    continue;
                }
                match store.type_of(imp.id) {
                    Some(ResourceType::ShaderProgramBuffer) => {
                        let program = store.load(imp.id)?;
                        println!("// ===== {} (via {via} slot {:#x}) =====", imp.id, imp.offset);
                        for sh in d3dasm::parse(program.body()) {
                            println!("{sh}");
                        }
                    }
                    Some(ResourceType::VertexProgramState) => {
                        let vps = store.load(imp.id)?;
                        queue.extend(vps.imports.iter().map(|i| (format!("VertexProgramState {}", imp.id), *i)));
                    }
                    _ => {}
                }
            }
        }
        Cmd::Damage { car, level, preview } => {
            let store = AssetStore::new(game)?;
            damage::run(&store, &damage::Options { car: &car, levels: &level, preview: preview.as_deref() })?
        }
        Cmd::VerifyHandling { csv, vehicle, wheels } => verify_handling::verify_handling(
            &AssetStore::new(game)?,
            csv.as_path(),
            vehicle.as_str(),
            wheels.as_deref(),
        )?,
        Cmd::Mods { car } => {
            let store = AssetStore::new(game)?;
            let bundle = if car.to_ascii_uppercase().ends_with(".BNDL") {
                car
            } else {
                let v = store.find_vehicle(&car)?.with_context(|| format!("no car matches {car}"))?;
                println!("{} ({}, id {})", v.name, v.year, v.id);
                v.hi_bundle.context("car has no bundle")?
            };
            let b = store.bundle(&bundle).with_context(|| format!("no bundle {bundle}"))?;
            const OPS: [&str; 3] = ["x", "+", "="];
            for (i, cat) in mw_assets::mods::performance_upgrades(&store, &b)?.iter().enumerate() {
                println!("category {i}");
                for c in cat {
                    let name = c.name.map_or("-".into(), |n| n.to_string());
                    let extra = match (&c.nitrous, c.tyre) {
                        (Some(n), _) => format!(" nitrous params {n}"),
                        (_, Some(t)) => format!(" tyre {t}"),
                        _ => String::new(),
                    };
                    let changes: Vec<String> = c
                        .modifications
                        .iter()
                        .map(|m| {
                            format!(
                                "{} {}{}",
                                mw_assets::mods::param_name(m.param),
                                OPS.get(m.op as usize).unwrap_or(&"?"),
                                m.value
                            )
                        })
                        .collect();
                    println!("  slot {:08x} {:<16} name {name}{extra}: {}", c.slot, c.kind, changes.join(", "));
                }
            }
        }
        Cmd::Handling { car } => {
            let store = AssetStore::new(game)?;
            let bundle = if car.to_ascii_uppercase().ends_with(".BNDL") {
                car
            } else {
                let v = store.find_vehicle(&car)?.with_context(|| format!("no car matches {car}"))?;
                println!("{} ({}, id {})", v.name, v.year, v.id);
                v.hi_bundle.context("car has no bundle")?
            };
            let hd = mw_assets::handling::VehicleHandling::for_vehicle(&store, &bundle)?;
            println!("{hd:#?}");
            if !hd.torque_curve.is_empty() {
                let max = hd.redline_rpm.unwrap_or(8000.0);
                let samples: Vec<String> =
                    (0..=8).map(|i| max * i as f32 / 8.0).map(|r| format!("{r:.0}:{:.0}", hd.torque_at(r))).collect();
                println!("torque (rpm:Nm): {}", samples.join("  "));
            }
        }
        Cmd::Surfaces => props::surfaces(&AssetStore::new(game)?)?,
        Cmd::CameraTake { take, step } => {
            camera_take::run(&open_bundle(&game, "CAMERAS.BNDL")?, take.as_deref(), step)?
        }
        Cmd::Freeroam { list, kind, all_kinds } => freeroam(&AssetStore::new(game)?, list, kind.as_deref(), all_kinds)?,
        Cmd::PlacementCheck { bundles, top, groups, view, column, owners } => {
            let store = AssetStore::new(game)?;
            if owners {
                placement::owners(&store)?
            } else if let Some(c) = column {
                placement::column(&store, &c)?
            } else if let Some(v) = view {
                placement::view(&store, &v, top)?
            } else if groups {
                placement::groups(&store, &bundles)?
            } else {
                placement::run(&store, &bundles, top)?
            }
        }
        Cmd::Props { bundles, verbose } => props::run(&AssetStore::new(game)?, &bundles, verbose)?,
        Cmd::Vehicles => {
            let store = AssetStore::new(game)?;
            let lang = store.language(mw_assets::DEFAULT_LANGUAGE)?;
            for v in store.vehicles(&lang)? {
                println!("{:>8}  {:<4}  {:<40} {}", v.id, v.year, v.name, v.hi_bundle.as_deref().unwrap_or("-"));
            }
        }
        Cmd::Collision { bundle, out } => {
            use std::fmt::Write as _;
            let b = open_bundle(&game, &bundle)?;
            std::fs::create_dir_all(&out)?;
            let (mut obj, mut tris, mut base) = (String::new(), Vec::new(), 1u32);
            for e in b.entries_of_type(ResourceType::PolygonSoupList) {
                let list = fmt::PolygonSoupList::parse(&b.load(e)?)?;
                println!("{}: {} soups, bounds {:?} .. {:?}", e.id, list.soups.len(), list.min, list.max);
                let (verts, faces) = list.triangles();
                for v in &verts {
                    writeln!(obj, "v {} {} {}", v[0], v[1], v[2])?;
                }
                for f in &faces {
                    writeln!(obj, "f {} {} {}", f[0] + base, f[1] + base, f[2] + base)?;
                    tris.push(preview::Tri { p: f.map(|i| verts[i as usize]), color: [200, 170, 120] });
                }
                base += verts.len() as u32;
            }
            std::fs::write(out.join("collision.obj"), obj)?;
            preview::render_views(&tris, 512).save(out.join("preview.png"))?;
            println!("{} triangles -> {}", tris.len(), out.display());
        }
        Cmd::Genesys { target, r#type, max_items } => {
            let store = AssetStore::new(game)?;
            let objects: Vec<(ResourceId, mw_bundle::Resource)> = match ResourceId::parse(&target) {
                _ if target == "all" => {
                    // Every GenesysObject in the game whose type name matches --type.
                    let want = r#type.clone().unwrap_or_default();
                    let mut out = Vec::new();
                    for rel in store.bundle_paths().to_vec() {
                        let Some(b) = store.bundle(&rel) else { continue };
                        for e in b.entries_of_type(ResourceType::GenesysObject) {
                            let res = b.load(e)?;
                            let matches = res
                                .import_at(0)
                                .and_then(|t| store.genesys_type(t))
                                .is_some_and(|t| t.name.contains(want.as_str()));
                            if matches {
                                println!("# {} in {}", e.id, rel.display());
                                out.push((e.id, res));
                            }
                        }
                        store.evict_all();
                    }
                    out
                }
                Some(id) if !target.to_ascii_uppercase().ends_with(".BNDL") => vec![(id, store.load(id)?)],
                _ => {
                    let b = store.bundle(&target).with_context(|| format!("bundle not found: {target}"))?;
                    b.entries_of_type(ResourceType::GenesysObject)
                        .map(|e| Ok((e.id, b.load(e)?)))
                        .collect::<Result<_>>()?
                }
            };
            for (id, res) in objects {
                match store.decode_genesys(&res) {
                    Ok(obj) => {
                        if r#type.as_ref().is_some_and(|t| !obj.type_name.contains(t.as_str())) {
                            continue;
                        }
                        println!("== {id} {}", obj.type_name);
                        print_genesys(&obj, 1, max_items);
                    }
                    Err(e) => println!("== {id} <{e}>"),
                }
            }
        }
        Cmd::GenesysCrack { words, max_words, styles, r#type, include_named } => {
            let store = AssetStore::new(game)?;
            let opt = crack::Options { words, max_words, styles, type_filter: r#type, include_named };
            crack::run(&store, &opt)?;
        }
        Cmd::GenesysTypes => {
            let store = AssetStore::new(game)?;
            let mut seen = std::collections::HashSet::new();
            for rel in store.bundle_paths().to_vec() {
                let Some(b) = store.bundle(&rel) else { continue };
                for e in b.entries_of_type(ResourceType::GenesysType) {
                    if !seen.insert(e.id) {
                        continue;
                    }
                    let Some(t) = store.genesys_type(e.id) else { continue };
                    println!("T\t{}\t{}\t{:?}\t{}", t.id, t.name, t.kind, t.size);
                    for (i, f) in t.fields.iter().enumerate() {
                        let ty = f.ty.and_then(|id| store.genesys_type(id)).map_or_else(|| "?".into(), |t| t.name);
                        println!(
                            "F\t{i}\t{:08x}\t{}\t{}\t{}\t{:x}\t{ty}",
                            f.name_hash, f.offset, f.size, f.count, f.flags
                        );
                    }
                }
                store.evict_all();
            }
        }
        Cmd::Audio { source, out } => {
            std::fs::create_dir_all(&out)?;
            let report = |name: &str, pcm: &mw_audio::Pcm| {
                let rms = (pcm.samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                    / pcm.samples.len().max(1) as f64)
                    .sqrt();
                println!(
                    "{name}: {} ch, {} Hz, {:.2} s, rms {rms:.0}",
                    pcm.channels,
                    pcm.sample_rate,
                    pcm.duration_secs()
                );
            };
            if source.to_ascii_uppercase().ends_with(".SPS") {
                let path = game.path(&source).with_context(|| format!("not found: {source}"))?;
                let pcm = mw_audio::sps::decode(&std::fs::read(&path)?)?;
                report(&source, &pcm);
                let name = Path::new(&source).file_stem().unwrap_or_default().to_string_lossy().into_owned();
                std::fs::write(out.join(format!("{name}.wav")), pcm.to_wav())?;
            } else {
                let b = open_bundle(&game, &source)?;
                for e in b.entries_of_type(ResourceType::Ginsu) {
                    match mw_audio::ginsu::Ginsu::parse(b.load(e)?.header()) {
                        Ok(g) => {
                            report(
                                &format!(
                                    "{} ginsu {:.0}-{:.0} rpm ({}), {} grains",
                                    e.id,
                                    g.min_rpm,
                                    g.max_rpm,
                                    if g.rising() { "rising" } else { "falling" },
                                    g.grains.len()
                                ),
                                &g.pcm,
                            );
                            std::fs::write(out.join(format!("{}_ginsu.wav", e.id)), g.pcm.to_wav())?;
                            // Tables alongside, for analysis: header line, then the RPM table (recording
                            // position per evenly spaced RPM, signed) and the grain boundaries.
                            let mut tables = format!(
                                "min_rpm {} max_rpm {} sample_rate {} samples {}\nrpm_table",
                                g.min_rpm,
                                g.max_rpm,
                                g.pcm.sample_rate,
                                g.pcm.samples.len()
                            );
                            for v in &g.rpm_table {
                                tables += &format!(" {}", *v as i32);
                            }
                            tables += "\ngrains";
                            for v in &g.grains {
                                tables += &format!(" {v}");
                            }
                            tables.push('\n');
                            std::fs::write(out.join(format!("{}_ginsu.txt", e.id)), tables)?;
                        }
                        Err(err) => println!("{} ginsu: {err}", e.id),
                    }
                }
                for e in b.entries_of_type(ResourceType::Wave) {
                    let res = b.load(e)?;
                    match res.header().get(0x80..).map(mw_audio::sps::decode) {
                        Some(Ok(pcm)) => {
                            report(&format!("{} wave", e.id), &pcm);
                            std::fs::write(out.join(format!("{}_wave.wav", e.id)), pcm.to_wav())?;
                        }
                        Some(Err(err)) => println!("{} wave: {err}", e.id),
                        None => println!("{} wave: too short", e.id),
                    }
                }
            }
        }
        Cmd::GinsuResynth { bundle, index, out, hold, seconds } => {
            let b = open_bundle(&game, &bundle)?;
            let e = b.entries_of_type(ResourceType::Ginsu).nth(index).context("no such Ginsu")?;
            let g = std::sync::Arc::new(mw_audio::ginsu::Ginsu::parse(b.load(e)?.header())?);
            let sr = g.pcm.sample_rate;
            let mut synth = mw_audio::ginsu::GinsuSynth::new(g.clone());
            let samples = match hold {
                Some(rpm) => synth.render(|_| rpm, (seconds * sr as f32) as usize),
                None => {
                    let n = g.pcm.samples.len();
                    synth.render(|i| g.rpm_at_position(i as f32), n)
                }
            };
            let samples = samples.iter().map(|s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16).collect();
            std::fs::write(&out, mw_audio::Pcm { channels: 1, sample_rate: sr, samples }.to_wav())?;
            println!("{} {:.0}-{:.0} rpm -> {}", e.id, g.min_rpm, g.max_rpm, out.display());
        }
        Cmd::EngineSound { car, out, trace, legacy, dry, competitor } => {
            let store = AssetStore::new(game)?;
            let v = store.find_vehicle(&car)?.with_context(|| format!("no car matches {car}"))?;
            let b = store.bundle(v.hi_bundle.as_deref().context("no bundle")?).context("bundle")?;
            let h = mw_assets::handling::VehicleHandling::from_bundle(&store, &b).unwrap_or_default();
            let (idle, redline) = (h.idle_rpm.unwrap_or(900.0), h.redline_rpm.unwrap_or(7000.0));
            // A recorded trace (`seconds rpm throttle [gear]`, stepwise as the game updates it), or a 4 s
            // rev-up on full throttle and a 4 s coast down, in physics RPM.
            // Columns after the speed (`MW_ENGINE_TRACE` of the game): engine throttle, clutch, gear-change clock,
            // shifts started, last shift up, cruising box, top gears; without them the driver makes them up.
            let points: Vec<([f32; 5], Option<mw_assets::engine_driver::DriveState>)> = match &trace {
                Some(p) => std::fs::read_to_string(p)?
                    .lines()
                    .filter_map(|l| {
                        let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                        (v.len() >= 3).then(|| {
                            let drive = (v.len() >= 12).then(|| mw_assets::engine_driver::DriveState {
                                engine_throttle: v[5],
                                clutch: v[6],
                                gear_change_elapsed: v[7],
                                shifts_started: v[8] as u32,
                                last_shift_up: v[9] > 0.5,
                                cruising: v[10] > 0.5,
                                top_gears: v[11] > 0.5,
                            });
                            (
                                [v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0), v.get(4).copied().unwrap_or(0.0)],
                                drive,
                            )
                        })
                    })
                    .collect(),
                None => vec![],
            };
            let duration = points.last().map_or(8.0, |p| p.0[0]);
            let input_at = |t: f32, k: &mut usize| -> ([f32; 4], Option<mw_assets::engine_driver::DriveState>) {
                if !points.is_empty() {
                    while *k + 1 < points.len() && points[*k + 1].0[0] <= t {
                        *k += 1;
                    }
                    let p = &points[*k];
                    ([p.0[1], p.0[2], p.0[3], p.0[4]], p.1)
                } else if t < 4.0 {
                    ([idle + (redline - idle) * t / 4.0, 1.0, 1.0, 0.0], None)
                } else {
                    ([redline - (redline - idle) * (t - 4.0) / 4.0, 0.0, 1.0, 0.0], None)
                }
            };
            let voice = if competitor {
                let c = store.competitor_engine(v.id)?.context("no competitor sound")?;
                println!("competitor engine: 3D {:?}", c.spatial);
                Some(mw_assets::engine_sound::EngineVoice::Bank(c.engine))
            } else if legacy {
                store.engine_voice(&b)?.map(|(l, g)| mw_assets::engine_sound::EngineVoice::Layers(l, g))
            } else {
                store.engine(&b)?
            };
            let mut samples = Vec::new();
            let mut k = 0;
            match voice.context("no engine sound")? {
                mw_assets::engine_sound::EngineVoice::Bank(eb) => {
                    let rate = mw_audio::sequence::OUTPUT_RATE as usize;
                    println!(
                        "engine bank: {} sequences, {} channels, rpm axis {:.0}-{:.0}, audible {:.0}-{:.0}, dominant order {}, physics {idle:.0}-{redline:.0} rpm, gear-change variants {:?}",
                        eb.bank.sequences.len(),
                        eb.bank.channels.len(),
                        eb.rpm_range.0,
                        eb.rpm_range.1,
                        eb.audible_range.0,
                        eb.audible_range.1,
                        eb.harmonic_order,
                        eb.gear_changes
                    );
                    for s in &eb.bank.sequences {
                        println!(
                            "  sequence {:016x} {:?} {}..{}: {} items",
                            s.id,
                            s.binding,
                            s.min,
                            s.max,
                            s.items.len()
                        );
                    }
                    let params = std::sync::Arc::new(mw_audio::sequence::Params::new(eb.bank.param_names()));
                    let mut driver = mw_assets::engine_driver::EngineDriver::new(&eb, params.clone());
                    driver.set_pop_params(mw_assets::engine_driver::PopParams::from_handling(&h));
                    if let Some(v) = h.param("Engine.937338e7") {
                        driver.set_speed_load(v);
                    }
                    let mut player = mw_audio::sequence::Player::new(eb.bank.clone(), params);
                    player.dry = dry;
                    player.bypass = mw_audio::sequence::Bypass::from_env();
                    player.enable_meter();
                    // The game updates the engine at its physics rate; 64 Hz here. `MW_AUDIO_BURST=<samples>`
                    // mimics the game, where the audio thread renders a device buffer at a time: the values the
                    // player sees then only change once per buffer.
                    let burst: usize = std::env::var("MW_AUDIO_BURST").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let tick = if burst > 0 { burst } else { rate / 64 };
                    for i in 0..(rate as f32 * duration) as usize {
                        if i % tick == 0 {
                            let t = i as f32 / rate as f32;
                            let ([rpm, throttle, gear, speed], drive) = input_at(t, &mut k);
                            let input = mw_assets::engine_driver::EngineInput {
                                rpm,
                                throttle,
                                gear: gear as i32,
                                speed_mph: speed.abs() / 0.447_04,
                                idle_rpm: idle,
                                redline_rpm: redline,
                                top_speed_mph: h.top_speed_mph.unwrap_or(180.0),
                                drive,
                            };
                            driver.update(tick as f32 / rate as f32, &input);
                            sound_trace(t, rpm, driver.state());
                        }
                        samples.push((player.next_sample() * 32767.0).clamp(-32768.0, 32767.0) as i16);
                    }
                    if std::env::var_os("MW_AUDIO_METER").is_some() {
                        let n = samples.len().max(1) as f64;
                        for (id, e) in player.meter() {
                            let chain: Vec<String> = eb
                                .bank
                                .channels
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| {
                                    c.plugins
                                        .iter()
                                        .map(|p| {
                                            let f: Vec<String> =
                                                p.fields.iter().map(|(h, v)| format!("{h:08x}={v}")).collect();
                                            format!("{:?}[{}]({})", p.kind, p.uid, f.join(","))
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            println!(
                                "  channel {id:016x}: {:.1} dB  {}",
                                10.0 * (e / n + 1e-12).log10(),
                                chain.join(" > ")
                            );
                        }
                    }
                    let pcm = mw_audio::Pcm { channels: 1, sample_rate: rate as u32, samples };
                    std::fs::write(&out, pcm.to_wav())?;
                }
                mw_assets::engine_sound::EngineVoice::Layers(layers, master) => {
                    let rate = layers[0].ginsu.pcm.sample_rate as usize;

                    for l in &layers {
                        let dir = if l.ginsu.rising() { "rising" } else { "falling" };
                        println!(
                            "layer {:.0}-{:.0} rpm ({dir}), gain {:.3}, load curve {:?}",
                            l.ginsu.min_rpm, l.ginsu.max_rpm, l.gain, l.load_curve
                        );
                    }
                    let mut mixer = mw_audio::ginsu::EngineMixer::new(layers, master);
                    for i in 0..(rate as f32 * duration) as usize {
                        let t = i as f32 / rate as f32;
                        let ([rpm, load, _, _], _) = input_at(t, &mut k);
                        samples.push((mixer.next_sample(rpm, load) * 32767.0).clamp(-32768.0, 32767.0) as i16);
                    }
                    let pcm = mw_audio::Pcm { channels: 1, sample_rate: rate as u32, samples };
                    std::fs::write(&out, pcm.to_wav())?;
                }
            }
            println!("{}: {:.1} s -> {}", v.name, duration, out.display());
        }
        Cmd::TrafficSound { component, out, passby } => {
            let store = AssetStore::new(game)?;
            traffic_sound::run(&store, &component, out.as_deref(), passby)?;
        }
        Cmd::VehicleSound { out, dry, sweep, mph } => {
            use mw_assets::vehicle_driver::{VehicleDriver, VehicleInput, WheelSlip};
            let store = AssetStore::new(game)?;
            let bank = store.vehicle_sfx_bank()?;
            println!("vehicle bank: {} sequences, {} channels", bank.sequences.len(), bank.channels.len());
            for s in &bank.sequences {
                println!("  sequence {:016x} {:?} {}..{}: {} items", s.id, s.binding, s.min, s.max, s.items.len());
            }
            for (c, s) in &bank.cues {
                println!("  cue {c}: {} sequences", s.len());
            }
            if let Some(which) = sweep {
                let tyre = store.tyre_sound_params();
                if let Some(t) = &tyre {
                    println!("TyreSoundParams: {t:?}");
                }
                return skid_sweep(bank, tyre.unwrap_or_default(), &which, mph, dry, &out);
            }
            let params = std::sync::Arc::new(mw_audio::sfx_player::Params::new(bank.param_names()));
            let mut driver = VehicleDriver::new(params.clone());
            match store.tyre_sound_params() {
                Some(t) => driver.tyre = t,
                None => eprintln!("no TyreSoundParams; using the built-in copy"),
            }
            let mut player = mw_audio::sfx_player::Player::new(bank, params);
            player.dry = dry;
            // Scripted 12 s: accelerate to 60 mph, cruise, drift, wheelspin, locked brakes, impacts, a scrape,
            // then speed up to 160 mph for the wind.
            let rate = mw_audio::sfx_player::OUTPUT_RATE as usize;
            let tick = rate / 64;
            let mut samples = Vec::new();
            let mut fired = [false; 3];
            for i in 0..rate * 12 {
                if i % tick == 0 {
                    let t = i as f32 / rate as f32;
                    let mph = if t < 2.0 {
                        30.0 * t
                    } else if t < 10.0 {
                        60.0
                    } else {
                        60.0 + 50.0 * (t - 10.0)
                    };
                    let v = mph * 0.447_04;
                    // A 30° drift, wheelspin at 8 m/s, then locked wheels under full braking.
                    let (slip_angle, slip_speed, brake) = match t {
                        t if (2.5..4.5).contains(&t) => (30.0, 0.0, 0.0),
                        t if (5.0..6.0).contains(&t) => (0.0, 8.0, 0.0),
                        t if (6.5..7.5).contains(&t) => (0.0, -v, 1.0),
                        _ => (0.0, 0.0, 0.0),
                    };
                    let scrape = if (9.0..10.0).contains(&t) { 8.0 } else { 0.0 };
                    driver.update(
                        1.0 / 64.0,
                        &VehicleInput {
                            speed_mph: mph,
                            skid_wheel: Some(WheelSlip { slip_angle, slip_speed }),
                            brake,
                            handbrake: false,
                            scrape_speed: scrape,
                        },
                    );
                    for (k, (at, speed)) in [(8.0, 15.0), (8.5, 6.0), (8.8, 2.0)].into_iter().enumerate() {
                        if t >= at && !fired[k] {
                            fired[k] = true;
                            driver.impact(speed);
                        }
                    }
                }
                samples.push((player.next_sample() * 32767.0).clamp(-32768.0, 32767.0) as i16);
            }
            let pcm = mw_audio::Pcm { channels: 1, sample_rate: rate as u32, samples };
            std::fs::write(&out, pcm.to_wav())?;
            println!("12 s -> {}", out.display());
        }
        Cmd::ShaderUsage => {
            let store = AssetStore::new(game)?;
            let mut usage: std::collections::HashMap<String, (usize, usize, usize)> = Default::default();
            let mut seen = std::collections::HashSet::new();
            let units: Vec<_> = store
                .bundle_paths()
                .iter()
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
                .cloned()
                .collect();
            for u in units {
                let Some(b) = store.bundle(&u) else { continue };
                for p in store.bundle_placements(&b)? {
                    if !seen.insert(p.model) {
                        continue;
                    }
                    let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
                    for m in meshes {
                        let e = usage.entry(m.shader_name.clone()).or_default();
                        e.0 += m.mesh.indices.len() / 3;
                        e.1 += 1;
                        if m.diffuse.is_none() {
                            e.2 += 1;
                        }
                    }
                }
                store.evict_all();
            }
            let mut rows: Vec<_> = usage.into_iter().collect();
            rows.sort_by_key(|a| std::cmp::Reverse(a.1.0));
            for (name, (tris, meshes, untextured)) in rows {
                println!("{tris:>9} tris {meshes:>6} meshes {untextured:>6} without diffuse  {name}");
            }
        }
        Cmd::SamplerUsage { filter, textures } => {
            let store = AssetStore::new(game)?;
            // (shader, sampler, state summary) -> meshes
            let mut usage: std::collections::BTreeMap<(String, String, String), usize> = Default::default();
            let mut seen = std::collections::HashSet::new();
            let mut states = std::collections::HashMap::new();
            let units: Vec<_> = store
                .bundle_paths()
                .iter()
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
                .cloned()
                .collect();
            for u in units {
                let Some(b) = store.bundle(&u) else { continue };
                for p in store.bundle_placements(&b)? {
                    if !seen.insert(p.model) {
                        continue;
                    }
                    let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
                    for m in meshes {
                        if filter.as_ref().is_some_and(|f| !m.shader_name.contains(f.as_str())) {
                            continue;
                        }
                        let bound = if textures { &m.textures } else { &m.sampler_states };
                        for (name, id) in bound {
                            let summary = states
                                .entry(*id)
                                .or_insert_with(|| {
                                    let r = if textures {
                                        store.texture(*id).map(|t| {
                                            let full = 1 + t.width.max(t.height).max(1).ilog2();
                                            format!("{:?} mips {}/{full}", t.format, t.mip_count)
                                        })
                                    } else {
                                        store.sampler_state(*id).map(|s| s.summary())
                                    };
                                    r.unwrap_or_else(|e| format!("error {e}"))
                                })
                                .clone();
                            *usage.entry((m.shader_name.clone(), name.clone(), summary)).or_default() += 1;
                        }
                    }
                }
                store.evict_all();
            }
            for ((shader, sampler, state), n) in usage {
                println!("{n:>6}  {shader:<48} {sampler:<32} {state}");
            }
        }
        Cmd::Shaders => {
            let store = AssetStore::new(game)?;
            let b = store.bundle("SHADERS.BNDL").context("SHADERS.BNDL missing")?;
            let mut rows: Vec<String> = b
                .entries_of_type(ResourceType::Shader)
                .filter_map(|e| fmt::Shader::parse(&b.load(e).ok()?).ok())
                .map(|s| {
                    let samplers: Vec<&str> = s.samplers.iter().map(|x| x.name.as_str()).collect();
                    let params: Vec<String> =
                        s.parameters.iter().map(|p| format!("{}={:08x}", p.name, p.hash)).collect();
                    let layout = s
                        .vertex_descriptor
                        .and_then(|vd| store.vertex_descriptor(vd).ok())
                        .map(|vd| {
                            vd.attributes
                                .iter()
                                .map(|a| format!("{:?}:{:?}", a.semantic, a.format))
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    format!(
                        "{}: samplers [{}] params [{}] layout [{}]",
                        s.name,
                        samplers.join(", "),
                        params.join(", "),
                        layout
                    )
                })
                .collect();
            rows.sort();
            rows.iter().for_each(|r| println!("{r}"));
        }
        Cmd::World { out, bundles, size, obj, instances } => {
            export_world(&AssetStore::new(game)?, &bundles, &out, size, obj, &instances)?
        }
        Cmd::Events { id, png, all } => events::run(&AssetStore::new(game)?, id.as_deref(), png.as_deref(), all)?,
        Cmd::Traffic { png, at } => traffic(&AssetStore::new(game)?, png.as_deref(), &at)?,
        Cmd::PaintMap { id, png, at } => paint_map(&AssetStore::new(game)?, &id, png.as_deref(), &at)?,
    }
    Ok(())
}

/// `vehicle-sound --sweep`: one skid intensity binding swept 0 → 1 at a fixed speed, everything else at rest.
/// Prints the swept sequence's wave layers (envelope × gain) against the intensity, then renders the sweep:
/// 1 s at 0, a 10 s ramp, 2 s at 1.
fn skid_sweep(
    bank: mw_audio::sfx_player::Bank,
    tyre: mw_assets::vehicle_driver::TyreSoundParams,
    which: &str,
    mph: f32,
    dry: bool,
    out: &Path,
) -> Result<()> {
    use mw_audio::sfx_player::{ItemKind, OUTPUT_RATE, Params, Player};
    let suffix = match which {
        "lat" => "LatSlipIntensityForSkidSound",
        "spin" => "SpinIntensityForSkidSound",
        "braking" => "SpinBrakingIntensityForSkidSound",
        _ => bail!("--sweep takes lat, spin or braking"),
    };
    let params = std::sync::Arc::new(Params::new(bank.param_names()));
    let names: Vec<String> = params.names().to_vec();
    let set = |end: &str, v: f32| {
        for n in names.iter().filter(|n| n.rsplit('.').next() == Some(end)) {
            params.set(n, v);
        }
    };
    let speed_of = |b: &str| match b.rsplit('.').next() {
        Some("GetPlayerSpeed" | "GetSpeedMph") => Some(mph),
        _ => None,
    };
    if let Some(seq) = bank.sequences.iter().find(|s| s.binding.ends_with(suffix)) {
        println!("{:016x} {}: wave layers at {mph} mph (envelope x gain / pitch)", seq.id, seq.binding);
        // The physical input that gives intensity x at this speed (the steady state of FUN_009d1bf0's ramps).
        let (ramp, unit) = match which {
            "lat" => (tyre.lat_slip, "deg"),
            "spin" => (tyre.long_spin, "mph"),
            _ => (tyre.long_spin_braking, "pedal"),
        };
        let sf = if ramp.full_speed_mph > 0.0 { (mph / ramp.full_speed_mph).min(1.0) } else { 1.0 };
        let sf = if which == "braking" { sf * 0.5 } else { sf };
        print!("  x     input({unit})");
        let layers: Vec<_> = seq
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| match &it.kind {
                ItemKind::Wave { waves, gain, pitch, .. } => Some((i, it, waves, gain, pitch)),
                _ => None,
            })
            .collect();
        for (i, it, waves, ..) in &layers {
            let looping = if waves.iter().all(|w| w.looping) { "loop" } else { "shot" };
            print!(" [{i}]{:.2}-{:.2}{looping}", it.start, it.end);
        }
        println!();
        for k in 0..=20 {
            let x = k as f32 / 20.0;
            let need = x / sf.max(1e-6);
            let input = if need <= 1.0 {
                format!("{:>9.2}", ramp.start + (ramp.full - ramp.start) * need.powf(1.0 / ramp.curve_power.max(1e-3)))
            } else {
                format!("{:>9}", "-")
            };
            print!("  {x:.2} {input}");
            for (_, it, _, gain, pitch) in &layers {
                let t = if it.end > it.start { (x - it.start) / (it.end - it.start) } else { 0.0 };
                let g = it.envelope(x) * gain.eval(t, speed_of(&gain.binding));
                let p = pitch.eval(t, speed_of(&pitch.binding));
                print!(" {g:>6.3}/{p:<4.2}{:>6}", "");
            }
            println!();
        }
    }
    let mut player = Player::new(bank, params.clone());
    player.dry = dry;
    set("EngineOn", 1.0);
    set("GetIsLocalPlayer", 1.0);
    let rate = OUTPUT_RATE as usize;
    let tick = rate / 64;
    let mut samples = Vec::new();
    for i in 0..rate * 13 {
        if i % tick == 0 {
            let t = i as f32 / rate as f32;
            set("GetPlayerSpeed", mph);
            set("GetSpeedMph", mph);
            set(suffix, ((t - 1.0) / 10.0).clamp(0.0, 1.0));
        }
        samples.push((player.next_sample() * 32767.0).clamp(-32768.0, 32767.0) as i16);
    }
    let pcm = mw_audio::Pcm { channels: 1, sample_rate: rate as u32, samples };
    std::fs::write(out, pcm.to_wav())?;
    println!("13 s sweep of {suffix} at {mph} mph -> {}", out.display());
    Ok(())
}

fn traffic(store: &AssetStore, png: Option<&Path>, at: &[f32]) -> Result<()> {
    let t = store.traffic_lanes()?;
    if let [x, z] = at[..] {
        let data = store.traffic_data()?;
        for l in t.lanes_near(x, z, 80.0) {
            let s = &t.hulls[l.hull].sections[l.section];
            let f = data.flows.get(s.flow as usize).copied().unwrap_or_default();
            let (a, b) = (l.points[0], l.points[l.points.len() - 1]);
            println!(
                "hull {:3} section {:3}: ({:.0}, {:.1}, {:.0}) -> ({:.0}, {:.1}, {:.0}), {:.0} m, {:.0} mph, {:.0}/min, fwd {:?} back {:?}{}",
                l.hull,
                l.section,
                a[0],
                a[1],
                a[2],
                b[0],
                b[1],
                b[2],
                l.length,
                f.speed_mph,
                f.density,
                s.forward,
                s.backward,
                if s.stop_line.is_some() { ", stop line" } else { "" }
            );
        }
        return Ok(());
    }
    let sum = |f: &dyn Fn(&mw_formats::TrafficHull) -> usize| t.hulls.iter().map(f).sum::<usize>();
    println!(
        "version {}, {} hulls ({} with lanes), {} sections, {} rungs, {} spans, {} neighbours, {} junctions, \
         {} stop lines, {} static vehicles",
        t.version,
        t.hulls.len(),
        t.hulls.iter().filter(|h| !h.sections.is_empty()).count(),
        sum(&|h| h.sections.len()),
        sum(&|h| h.rungs.len()),
        sum(&|h| h.spans.len()),
        sum(&|h| h.neighbours.len()),
        sum(&|h| h.junctions.len()),
        sum(&|h| h.stop_lines.len()),
        sum(&|h| h.static_vehicles.len()),
    );
    let length: f32 = t.hulls.iter().flat_map(|h| &h.sections).map(|s| s.length).sum();
    println!("total lane length {:.1} km", length / 1000.0);
    // Traffic-light junctions: phase cycle and the stop lines each light group holds (checked against the
    // hulls' stop line lists and the sections that own them).
    let (mut bad_refs, mut owned) = (0, 0);
    for (hi, hull) in t.hulls.iter().enumerate() {
        for (ji, j) in hull.junctions.iter().enumerate() {
            let cycle: Vec<String> =
                j.durations.iter().zip(&j.masks).map(|(d, m)| format!("{d:.1}s:{m:02x}")).collect();
            let groups: Vec<String> = j
                .groups
                .iter()
                .map(|g| {
                    let lines: Vec<String> = g.stop_lines.iter().map(|(h, s)| format!("{h}/{s}")).collect();
                    format!("[{}]", lines.join(" "))
                })
                .collect();
            // Where: the first rung of the section owning the first stop line.
            let at = j.groups.iter().flat_map(|g| &g.stop_lines).next().and_then(|&(h, line)| {
                let hull = t.hulls.get(h as usize)?;
                let sec = hull.sections.iter().find(|s| s.stop_line == Some(line))?;
                let r = hull.rungs.get(sec.first_rung as usize)?;
                Some(format!("({:.0}, {:.0}, {:.0})", r[0][0], r[0][1], r[0][2]))
            });
            println!(
                "  junction {hi}.{ji} at {}: cycle {} groups {}",
                at.unwrap_or_default(),
                cycle.join(" "),
                groups.join(" ")
            );
            for &(h, s) in j.groups.iter().flat_map(|g| &g.stop_lines) {
                let Some(target) = t.hulls.get(h as usize) else { continue };
                if s as usize >= target.stop_lines.len() {
                    bad_refs += 1;
                } else if target
                    .sections
                    .iter()
                    .any(|sec| sec.stop_line.is_some_and(|f| (f..f + sec.stop_line_count.max(1)).contains(&s)))
                {
                    owned += 1;
                }
            }
        }
    }
    println!("junction stop line references: {owned} owned by a section, {bad_refs} out of range");
    // Query check at the docs/viewpoints.md road spot.
    let near = t.lanes_near(-1131.0, 1627.0, 200.0);
    println!("{} lane sections within 200 m of (-1131, 1627)", near.len());
    let mut per_flow = std::collections::BTreeMap::<u8, (usize, f32)>::new();
    for s in t.hulls.iter().flat_map(|h| &h.sections) {
        let e = per_flow.entry(s.flow).or_default();
        e.0 += 1;
        e.1 += s.length;
    }
    let id = |r: &Option<ResourceId>| r.map_or("-".to_string(), |r| r.to_string());
    println!("{} flows (TrafficFlow):", t.flows.len());
    for (i, f) in t.flows.iter().enumerate() {
        let (n, len) = per_flow.get(&(i as u8)).copied().unwrap_or_default();
        println!("  [{i:2}] {}  {n:5} sections {:7.1} km", id(f), len / 1000.0);
    }
    println!("{} flow types (TrafficFlowType):", t.flow_types.len());
    for (i, f) in t.flow_types.iter().enumerate() {
        println!("  [{i:2}] {}", id(f));
    }
    let ids: Vec<String> =
        t.vehicle_ids.iter().map(|v| format!("{:016x}", 0x0100_0000_0000_0000u64 | *v as u64)).collect();
    println!("{} vehicle types (VehicleComponent): {}", ids.len(), ids.join(" "));
    // The decoded rule data the runtime uses (mw_assets::traffic).
    let data = store.traffic_data()?;
    for (i, v) in data.vehicles.iter().enumerate() {
        println!(
            "  vehicle {i}: {} ({} paints), box centre {:?} half {:?}, {:.0} kg{}",
            v.bundle,
            v.paints.len(),
            v.box_centre,
            v.half_extents,
            v.mass,
            if v.trailer { ", trailer" } else { "" }
        );
    }
    for (i, f) in data.flow_types.iter().enumerate() {
        let e: Vec<String> = f.iter().map(|e| format!("{}:{:.2}", e.vehicle, e.proportion)).collect();
        println!("  flow type {i}: {}", e.join(" "));
    }
    let f: Vec<String> = data.flows.iter().map(|f| format!("{:.0}mph/{:.0}", f.speed_mph, f.density)).collect();
    println!("  flows (speed / cars per minute): {}", f.join(" "));
    println!("  system: {:?}", data.system);
    println!("  params: {:?}", data.params);
    println!("  fuzzy: {:?}", data.fuzzy);
    if let Some(png) = png {
        let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in t.hulls.iter().flat_map(|h| &h.rungs).flatten() {
            (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[2]), x1.max(p[0]), z1.max(p[2]));
        }
        let scale = 4096.0 / (x1 - x0).max(z1 - z0);
        let (w, h) = (((x1 - x0) * scale) as u32 + 1, ((z1 - z0) * scale) as u32 + 1);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([16, 16, 20]));
        let mut plot = |x: f32, z: f32, c: [u8; 3]| {
            let (px, pz) = (((x - x0) * scale) as u32, ((z - z0) * scale) as u32);
            if px < w && pz < h {
                img.put_pixel(px, pz, image::Rgb(c));
            }
        };
        for hull in &t.hulls {
            // Cell outlines, dim.
            for (k, a) in hull.points.iter().enumerate() {
                let b = hull.points[(k + 1) % hull.points.len()];
                for s in 0..200 {
                    let f = s as f32 / 200.0;
                    plot(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, [50, 50, 60]);
                }
            }
            for s in &hull.sections {
                let hue = (s.flow as f32 * 0.618).fract();
                let c = if s.stop_line.is_some() {
                    [255, 40, 40]
                } else {
                    [(120.0 + 135.0 * hue) as u8, (255.0 - 150.0 * hue) as u8, (90.0 + 160.0 * (1.0 - hue)) as u8]
                };
                let rungs = &hull.rungs[s.first_rung as usize..][..s.rung_count as usize];
                for pair in rungs.windows(2) {
                    let mid = |r: &[[f32; 3]; 2]| [(r[0][0] + r[1][0]) * 0.5, (r[0][2] + r[1][2]) * 0.5];
                    let (a, b) = (mid(&pair[0]), mid(&pair[1]));
                    let steps = (((b[0] - a[0]).hypot(b[1] - a[1]) * scale) as usize).max(1);
                    for k in 0..=steps {
                        let f = k as f32 / steps as f32;
                        plot(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, c);
                    }
                }
            }
        }
        img.save(png)?;
        println!("wrote {} ({w}x{h}, x {x0:.0}..{x1:.0}, z {z0:.0}..{z1:.0}, +z down)", png.display());
    }
    Ok(())
}

fn paint_map(store: &AssetStore, id: &str, png: Option<&Path>, at: &[f32]) -> Result<()> {
    let id = ResourceId::parse(id).context("bad resource id")?;
    let map = mw_formats::WorldPaintMap::parse(&store.load(id)?)?;
    println!(
        "{}×{} cells of {} m, origin {:?}, {} entries",
        map.width,
        map.height,
        map.cell_size,
        map.origin,
        map.entries.len()
    );
    for (i, e) in map.entries.iter().enumerate() {
        println!("  [{i:2}] {:016x}", 0x0100_0000_0000_0000u64 | *e as u64);
    }
    // Every track unit's zone: which entries its polygon covers (sampled on a 25 m grid).
    let zones = store.zone_list()?;
    let mut unpainted = 0;
    for z in &zones.zones {
        let [x0, z0, x1, z1] = z.bounds;
        let mut seen = std::collections::BTreeMap::<u8, usize>::new();
        let mut samples = 0;
        let mut x = x0;
        while x <= x1 {
            let mut y = z0;
            while y <= z1 {
                if z.contains(x, y) {
                    samples += 1;
                    if let Some(i) = map.index_at(x, y) {
                        *seen.entry(i).or_default() += 1;
                    }
                }
                y += 25.0;
            }
            x += 25.0;
        }
        let painted: usize = seen.iter().filter(|(i, _)| **i != 0).map(|(_, n)| n).sum();
        if painted * 2 < samples {
            unpainted += 1;
        }
        println!("zone {:3}: {samples} samples, entries {seen:?}", z.id);
    }
    println!("{unpainted} of {} zones are mostly unpainted", zones.zones.len());
    for p in at.as_chunks::<2>().0 {
        let i = map.index_at(p[0], p[1]);
        let e = map.entry_at(p[0], p[1]).map(|e| format!("{:016x}", 0x0100_0000_0000_0000u64 | e as u64));
        println!("at {:.0},{:.0}: index {i:?} = {e:?}", p[0], p[1]);
    }
    if let Some(out) = png {
        // 16 m per pixel, +x right, +z down; zone outlines in white.
        let px = 16.0;
        let ox = map.origin[0] as f32 * map.cell_size;
        let oz = map.origin[1] as f32 * map.cell_size;
        let (w, h) = ((map.width as f32 * map.cell_size / px) as u32, (map.height as f32 * map.cell_size / px) as u32);
        let colour = |i: u8| {
            let h = (i as u32).wrapping_mul(2_654_435_761);
            if i == 0 { [0, 0, 0] } else { [(h >> 24) as u8 | 0x40, (h >> 16) as u8 | 0x40, (h >> 8) as u8 | 0x40] }
        };
        let mut img = image::RgbImage::new(w, h);
        for v in 0..h {
            for u in 0..w {
                let (x, z) = ((u as f32 + 0.5) * px - ox, (v as f32 + 0.5) * px - oz);
                img.put_pixel(u, v, image::Rgb(map.index_at(x, z).map_or([0, 0, 0], colour)));
            }
        }
        for z in &zones.zones {
            for (a, b) in z.points.iter().zip(z.points.iter().cycle().skip(1)) {
                for t in 0..64 {
                    let f = t as f32 / 64.0;
                    let u = ((a[0] + (b[0] - a[0]) * f + ox) / px) as i64;
                    let v = ((a[1] + (b[1] - a[1]) * f + oz) / px) as i64;
                    if (0..w as i64).contains(&u) && (0..h as i64).contains(&v) {
                        img.put_pixel(u as u32, v as u32, image::Rgb([255, 255, 255]));
                    }
                }
            }
        }
        img.save(out)?;
    }
    Ok(())
}

fn export_textures(b: &Bundle, out: &Path, png: bool) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let mut n = 0;
    for e in b.entries_of_type(ResourceType::Texture) {
        let tex = fmt::Texture::parse(&b.load(e)?)?;
        let name = b.debug_name(e.id).map(sanitize).unwrap_or_else(|| e.id.to_string());
        std::fs::write(out.join(format!("{name}.dds")), tex.to_dds())?;
        if png {
            match tex.decode_rgba8(0) {
                Some((w, h, px)) => image::RgbaImage::from_raw(w, h, px)
                    .context("bad image size")?
                    .save(out.join(format!("{name}.png")))?,
                None => eprintln!("{}: cannot decode {:?}", e.id, tex.format),
            }
        }
        n += 1;
    }
    println!("exported {n} textures to {}", out.display());
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

fn export_models(store: &AssetStore, b: &Bundle, out: &Path, verbose: bool) -> Result<()> {
    let placements = match store.vehicle_placements(b)? {
        Some(p) => p,
        None => b
            .entries_of_type(ResourceType::Model)
            .map(|e| mw_assets::Placement {
                model: e.id,
                transform: mw_assets::IDENTITY,
                backdrop: false,
                overrides: None,
                object: None,
            })
            .collect(),
    };
    export_scene(store, &placements, out, verbose, 512, true)
}

fn export_world(store: &AssetStore, bundles: &[String], out: &Path, size: u32, obj: bool, which: &str) -> Result<()> {
    let mut placements = Vec::new();
    let names: Vec<String> = if bundles.is_empty() {
        store
            .bundle_paths()
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    } else {
        bundles.to_vec()
    };
    let mut layers = mw_assets::WorldLayers::default();
    for name in &names {
        let b = store.bundle(name).with_context(|| format!("bundle not found: {name}"))?;
        layers.add(store.bundle_layers(&b)?);
    }
    println!(
        "layers: {} instances, {} props, {} compounds, {} dynamics ({} unresolved), {} lights (not drawn)",
        layers.instances.len(),
        layers.props.len(),
        layers.compounds.len(),
        layers.dynamics.len(),
        layers.unresolved,
        layers.lights
    );
    {
        let unique: std::collections::HashMap<_, _> = layers.groundcover.iter().copied().collect();
        println!(
            "groundcover: {} collections, {} sprites (not in the placement count; the runtime draws them)",
            unique.len(),
            unique.values().map(|&n| n as u64).sum::<u64>()
        );
        let mut names: std::collections::BTreeMap<&str, usize> = Default::default();
        for o in layers.instances.iter().filter_map(|p| p.overrides.as_deref()) {
            for n in o.constants.iter().map(|c| &c.0).chain(o.textures.iter().map(|t| &t.0)) {
                *names.entry(n.as_str()).or_default() += 1;
            }
        }
        let overridden = layers.instances.iter().filter(|p| p.overrides.is_some()).count();
        println!("instance overrides: {overridden} instances {names:?}");
    }
    placements.extend(layers.into_placements());
    let total = placements.len();
    placements.retain(|p| match which {
        "main" => !p.backdrop,
        "backdrop" => p.backdrop,
        _ => true,
    });
    println!("{} bundles, {} of {total} placements ({which})", names.len(), placements.len());
    export_scene(store, &placements, out, false, size, obj)
}

/// Decode the LOD0 meshes of every placement and write an .obj plus a
/// software-rendered preview.
fn export_scene(
    store: &AssetStore,
    placements: &[mw_assets::Placement],
    out: &Path,
    verbose: bool,
    size: u32,
    write_obj: bool,
) -> Result<()> {
    use std::collections::HashMap;
    use std::fmt::Write as _;
    use std::io::Write as _;

    std::fs::create_dir_all(out)?;
    let mut obj: Box<dyn std::io::Write> = if write_obj {
        Box::new(std::io::BufWriter::new(std::fs::File::create(out.join("scene.obj"))?))
    } else {
        Box::new(std::io::sink())
    };
    let mut tris = Vec::new();
    let mut vbase = 1u32;
    let mut cache: HashMap<ResourceId, Vec<mw_assets::SceneMesh>> = HashMap::new();
    let mut errors = 0usize;
    for (pi, p) in placements.iter().enumerate() {
        cache.entry(p.model).or_insert_with(|| match store.model_meshes(p.model, 0) {
            Ok((m, errs)) => {
                errors += errs.len();
                if verbose {
                    errs.iter().for_each(|e| eprintln!("{e}"));
                }
                m
            }
            Err(e) => {
                errors += 1;
                if verbose {
                    eprintln!("model {}: {e}", p.model);
                }
                Vec::new()
            }
        });
        let xf = |v: &[f32; 4]| {
            let m = &p.transform;
            std::array::from_fn::<f32, 3, _>(|c| v[0] * m[0][c] + v[1] * m[1][c] + v[2] * m[2][c] + m[3][c])
        };
        for sm in &cache[&p.model] {
            let dm = &sm.mesh;
            if verbose {
                println!(
                    "{} mesh {}: {} verts, {} tris, shader {}, diffuse {:?}",
                    sm.renderable,
                    sm.mesh_index,
                    dm.positions.len(),
                    dm.indices.len() / 3,
                    sm.shader_name,
                    sm.diffuse
                );
                for (name, v) in &sm.parameters {
                    println!("    {name} = {v:?}");
                }
                for (name, t) in &sm.textures {
                    println!("    {name} -> {t}");
                }
            }
            let mut name = String::new();
            write!(name, "o p{pi}_{}_{}_{}", sm.renderable, sm.mesh_index, sm.shader_name)?;
            writeln!(obj, "{name}")?;
            let world: Vec<[f32; 3]> = dm.positions.iter().map(xf).collect();
            for v in &world {
                writeln!(obj, "v {} {} {}", v[0], v[1], v[2])?;
            }
            if let Some(uv) = dm.texcoord(0) {
                for t in uv {
                    writeln!(obj, "vt {} {}", t[0], 1.0 - t[1])?;
                }
            }
            let color = hash_color(&sm.shader_name);
            for t in dm.indices.as_chunks::<3>().0 {
                let [a, b, c] = [t[0] + vbase, t[1] + vbase, t[2] + vbase];
                if dm.texcoord(0).is_some() {
                    writeln!(obj, "f {a}/{a} {b}/{b} {c}/{c}")?;
                } else {
                    writeln!(obj, "f {a} {b} {c}")?;
                }
                tris.push(preview::Tri {
                    p: [world[t[0] as usize], world[t[1] as usize], world[t[2] as usize]],
                    color,
                });
            }
            vbase += dm.positions.len() as u32;
        }
    }
    obj.flush()?;
    preview::render_views(&tris, size).save(out.join("preview.png"))?;
    println!("{} triangles, {} decode errors -> {}", tris.len(), errors, out.display());
    Ok(())
}

fn hash_color(s: &str) -> [u8; 3] {
    let h = mw_bundle::ResourceId::from_name(s).0 as u32;
    [128 + (h & 0x7F) as u8, 128 + ((h >> 8) & 0x7F) as u8, 128 + ((h >> 16) & 0x7F) as u8]
}

fn open_bundle(game: &GameDir, path: &str) -> Result<Bundle> {
    let p = Path::new(path);
    let resolved = if p.exists() { Some(p.to_path_buf()) } else { game.path(path) };
    let Some(resolved) = resolved else { bail!("bundle not found: {path}") };
    Bundle::open(&resolved).with_context(|| format!("opening {}", resolved.display()))
}

fn list(b: &Bundle) {
    let h = &b.header;
    println!("flags {:?}, {} resources, streams {:?}", h.flags, h.resource_count, h.stream_names);
    println!("{:<18} {:<22} {:>10} {:>10} {:>4}  name", "id", "type", "chunk0", "chunk1", "imp");
    for e in &b.entries {
        println!(
            "{:<18} {:<22} {:>10} {:>10} {:>4}  {}",
            e.id.to_string(),
            e.resource_type().name(),
            e.uncompressed_size(0),
            e.uncompressed_size(1),
            e.import_count,
            b.debug_name(e.id).unwrap_or("")
        );
    }
}

fn extract(b: &Bundle, out: &Path, filter: Option<&str>) -> Result<()> {
    let filter = filter.map(|f| ResourceType::from_name(f).with_context(|| format!("unknown type {f}"))).transpose()?;
    for e in &b.entries {
        let ty = e.resource_type();
        if filter.is_some_and(|f| f != ty) {
            continue;
        }
        let res = b.load(e)?;
        let dir = out.join(ty.name().as_ref());
        std::fs::create_dir_all(&dir)?;
        for (i, c) in res.chunks.iter().enumerate().filter(|(_, c)| !c.is_empty()) {
            std::fs::write(dir.join(format!("{}.chunk{i}", e.id)), c)?;
        }
        if !res.imports.is_empty() {
            let text: String = res.imports.iter().map(|i| format!("{:#06x} {}\n", i.offset, i.id)).collect();
            std::fs::write(dir.join(format!("{}.imports.txt", e.id)), text)?;
        }
    }
    println!("extracted to {}", out.display());
    Ok(())
}

#[derive(Default)]
struct TypeStats {
    count: usize,
    bytes: [usize; 4],
}

fn verify(game: &GameDir) -> Result<()> {
    let bundles = game.files_with_extension("BNDL");
    let stats: Mutex<BTreeMap<ResourceType, TypeStats>> = Default::default();
    let failures: Vec<String> = bundles
        .par_iter()
        .flat_map(|rel| {
            let b = match Bundle::open(game.root().join(rel)) {
                Ok(b) => b,
                Err(e) => return vec![format!("{}: {e}", rel.display())],
            };
            let mut errs = Vec::new();
            for e in &b.entries {
                match b.load(e) {
                    Ok(r) => {
                        // Typed parsers that must accept every instance.
                        if e.resource_type() == ResourceType::BearEffect
                            && let Err(err) = fmt::BearEffect::parse(&r)
                        {
                            errs.push(format!("{}: {} {err}", rel.display(), e.id));
                        }
                        // AIData: parses, links are symmetric, every node lies in its grid cell's range.
                        if e.resource_type() == ResourceType::AIData {
                            match fmt::AiData::parse(&r) {
                                Err(err) => errs.push(format!("{}: {} {err}", rel.display(), e.id)),
                                Ok(ai) => {
                                    let asym = ai.nodes.iter().enumerate().any(|(i, n)| {
                                        n.links.iter().any(|&l| !ai.nodes[l as usize].links.contains(&(i as u16)))
                                    });
                                    let in_cells: usize = ai.cells.iter().map(|c| c.1 as usize).sum();
                                    if asym || in_cells != ai.nodes.len() {
                                        errs.push(format!(
                                            "{}: {} AIData: asymmetric links {asym}, {in_cells} nodes in cells of {}",
                                            rel.display(),
                                            e.id,
                                            ai.nodes.len()
                                        ));
                                    }
                                }
                            }
                        }
                        let mut s = stats.lock().unwrap();
                        let t = s.entry(e.resource_type()).or_default();
                        t.count += 1;
                        for i in 0..4 {
                            t.bytes[i] += r.chunks[i].len();
                        }
                    }
                    Err(err) => errs.push(format!("{}: {err}", rel.display())),
                }
            }
            errs
        })
        .collect();

    println!("{:<24} {:>8} {:>14} {:>14}", "type", "count", "chunk0 bytes", "chunk1 bytes");
    for (ty, s) in stats.into_inner().unwrap() {
        println!("{:<24} {:>8} {:>14} {:>14}", ty.name(), s.count, s.bytes[0], s.bytes[1]);
    }
    println!("{} bundles, {} failures", bundles.len(), failures.len());
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    if !failures.is_empty() {
        bail!("verification failed");
    }
    Ok(())
}

fn effect(store: &AssetStore, id: &str, raw: bool) -> Result<()> {
    use fmt::bear::Op;
    if id == "library" {
        let lib = store.vfx_library()?;
        let show = |o: Option<ResourceId>| o.map(|i| i.to_string()).unwrap_or_else(|| "-".into());
        for (i, s) in lib.surfaces.iter().enumerate() {
            for (cond, e) in [("dry", &s.dry), ("wet", &s.wet)] {
                println!(
                    "surface {i:2} {cond}: wheel {} tyre {} rims {} impacts front {} left {} rear {} bottom {}",
                    show(e.wheel),
                    show(e.player_wheels.default_tyre),
                    show(e.player_wheels.on_rims),
                    show(e.impacts[0]),
                    show(e.impacts[6]),
                    show(e.impacts[3]),
                    show(e.impacts[9]),
                );
            }
        }
        let list = |v: &[Option<ResourceId>]| v.iter().map(|&o| show(o)).collect::<Vec<_>>().join(" ");
        println!("vehicle impacts: {}", list(&lib.vehicle_impacts));
        println!("vehicle scrapes: {}", list(&lib.vehicle_scrapes));
        return Ok(());
    }
    if id == "all" {
        let b = store.bundle("GLOBALEFFECTS.BNDL").context("GLOBALEFFECTS.BNDL")?;
        for e in b.entries_of_type(ResourceType::BearEffect) {
            let fx = fmt::BearEffect::parse(&b.load(e)?)?;
            let layers: Vec<String> = fx
                .layers
                .iter()
                .map(|l| {
                    let r = l.renderer().map(|r| format!("{:02x}", r.kind)).unwrap_or_else(|| "-".into());
                    let s = l.spawner().map(|s| format!("{:02x}", s.kind)).unwrap_or_else(|| "-".into());
                    format!("{}(max {}, spawn {s}, render {r})", l.name, l.max_particles)
                })
                .collect();
            println!("{} {} layers: {}", e.id, fx.layers.len(), layers.join(" "));
        }
        return Ok(());
    }
    let rid = ResourceId::parse(id).context("invalid resource id")?;
    let fx = fmt::BearEffect::parse(&store.load(rid)?)?;
    println!("effect {}: {} layers", fx.id, fx.layers.len());
    for (i, p) in fx.params.iter().enumerate() {
        println!("  param {i}: {} kind {:#04x} offset {:#x} id {:#06x}", p.name, p.kind, p.offset, p.id);
    }
    for l in &fx.layers {
        println!(
            "  layer {} max {} flags {} sort {} bucket {} pre-roll {}",
            l.name, l.max_particles, l.flags, l.sort_group, l.particles_per_bucket, l.pre_roll
        );
        let ch: Vec<String> = l.channels.iter().map(|c| format!("{}:{}", c.name, c.kind)).collect();
        println!("    channels {}", ch.join(" "));
        for (k, a) in l.actions.iter().enumerate() {
            let desc = match &a.op {
                Op::Other => String::new(),
                op => format!("{op:?}"),
            };
            println!("    a{k:02} {:#04x} {desc}", a.kind);
            if raw {
                let words: Vec<String> = a
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|w| {
                        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                        let f = f32::from_bits(v);
                        if v != 0 && (1e-5..1e7).contains(&f.abs()) { format!("{f}") } else { format!("{v:#x}") }
                    })
                    .collect();
                println!("        {}", words.join(" "));
            }
        }
        for (g, group) in l.groups.iter().enumerate() {
            let Some(group) = group else { continue };
            for s in &group.sequences {
                let steps: Vec<String> = s
                    .steps
                    .iter()
                    .map(|st| {
                        if st.secondaries.is_empty() {
                            format!("a{:02}", st.action)
                        } else {
                            let sec: Vec<String> = st.secondaries.iter().map(|a| format!("a{a:02}")).collect();
                            format!("a{:02}[{}]", st.action, sec.join(" "))
                        }
                    })
                    .collect();
                println!("    group {g}{}: {}", if s.per_bucket { " per bucket" } else { "" }, steps.join(" "));
            }
        }
    }
    Ok(())
}

/// `shader-asm --exe`: the DXBC programs stored in `NFS13.exe`, addressed by their virtual address (as in
/// the decompiled dump). `.rdata` is not encrypted on disk, so the file is read as is; nothing is written.
fn exe_shader_asm(game: &GameDir, filter: &str) -> Result<()> {
    let path = game.path(mw_gamedir::EXECUTABLE).context("NFS13.exe not found")?;
    let data = std::fs::read(&path)?;
    let va_of = pe_file_offset_to_va(&data);
    let wanted = parse_int(filter).ok();
    for shader in d3dasm::parse(&data) {
        let va = va_of(shader.offset()).unwrap_or(0);
        let text = shader.to_string();
        let stage = text.lines().map(str::trim).find(|l| l.starts_with("ps_") || l.starts_with("vs_")).unwrap_or("?");
        if filter == "list" {
            let mut names = Vec::new();
            if let Some(rd) = shader.resource_def() {
                names.extend(rd.bindings.iter().filter(|b| !b.name.starts_with('$')).map(|b| b.name.to_string()));
                for cb in &rd.constant_buffers {
                    names.extend(cb.variables.iter().map(|v| v.name.to_string()));
                }
            }
            names.dedup();
            println!("{va:#010x} {stage:<6} {:>5} B  {}", shader.size(), names.join(" "));
        } else if filter == "all" || wanted == Some(va as usize) || (wanted.is_none() && text.contains(filter)) {
            println!("// ===== NFS13.exe {va:#010x} ({} bytes) =====\n{text}", shader.size());
        }
    }
    Ok(())
}

/// Maps a file offset in a 32-bit PE image to its virtual address (image base + section RVA).
fn pe_file_offset_to_va(data: &[u8]) -> impl Fn(usize) -> Option<u32> + '_ {
    let u16_at = |o: usize| data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let pe = u32_at(0x3c).unwrap_or(0) as usize;
    let sections = u16_at(pe + 6).unwrap_or(0) as usize;
    let optional = u16_at(pe + 20).unwrap_or(0) as usize;
    let base = u32_at(pe + 24 + 28).unwrap_or(0);
    let table: Vec<(u32, u32, u32)> = (0..sections)
        .filter_map(|i| {
            let o = pe + 24 + optional + 40 * i;
            Some((u32_at(o + 12)?, u32_at(o + 20)?, u32_at(o + 16)?))
        })
        .collect();
    move |offset| {
        let offset = u32::try_from(offset).ok()?;
        table
            .iter()
            .find(|&&(_, raw, len)| (raw..raw + len).contains(&offset))
            .map(|&(rva, raw, _)| base + rva + offset - raw)
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal integer.
fn parse_int(s: &str) -> Result<usize, std::num::ParseIntError> {
    match s.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => s.parse(),
    }
}

fn print_genesys(obj: &fmt::genesys::Object, depth: usize, max_items: usize) {
    use fmt::genesys::Value;
    let pad = "  ".repeat(depth);
    fn scalar(v: &Value) -> Option<String> {
        Some(match v {
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => format!("{f}"),
            Value::Bool(b) => b.to_string(),
            Value::Vector(v) => format!("{v:?}"),
            Value::Matrix(m) => format!("{m:?}"),
            Value::Handle(h) => format!("handle {}", h.map_or("-".into(), |h| h.to_string())),
            Value::Ref(r) => format!("ref {r}"),
            Value::Undecoded(e) => format!("<undecoded: {e}>"),
            Value::Null => "null".into(),
            Value::Str(s) => format!("{s:?}"),
            Value::Raw(b) => format!("raw {}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
            _ => return None,
        })
    }
    for (hash, v) in &obj.fields {
        // Known names print as-is, unknown ones as their hash.
        let h = fmt::genesys::field_name(*hash).map_or_else(|| format!("{hash:08x}"), str::to_string);
        match v {
            Value::Struct(o) => {
                println!("{pad}{h}: {} {{", o.type_name.trim_start_matches("Genesys.Gen."));
                print_genesys(o, depth + 1, max_items);
                println!("{pad}}}");
            }
            Value::Array(items) => {
                println!("{pad}{h}: [{} items]", items.len());
                for (i, it) in items.iter().take(max_items).enumerate() {
                    match it {
                        Value::Struct(o) => {
                            println!("{pad}  [{i}] {} {{", o.type_name.trim_start_matches("Genesys.Gen."));
                            print_genesys(o, depth + 2, max_items);
                            println!("{pad}  }}");
                        }
                        other => println!("{pad}  [{i}] {}", scalar(other).unwrap_or_default()),
                    }
                }
            }
            other => println!("{pad}{h}: {}", scalar(other).unwrap_or_default()),
        }
    }
}

#[cfg(target_os = "linux")]
fn attach(process: &str, pid: Option<u32>) -> Result<memscan::Process> {
    match pid {
        Some(pid) => memscan::Process::open(pid),
        None => memscan::Process::find(process),
    }
}

#[cfg(target_os = "linux")]
fn memscan_cmd(process: &str, pid: Option<u32>, watch: &Path, all_regions: bool) -> Result<()> {
    memscan::repl(&attach(process, pid)?, watch, all_regions)
}

#[cfg(target_os = "linux")]
fn tune_cmd(process: &str, pid: Option<u32>, preset: &Path, apply: bool, print: bool) -> Result<()> {
    tune::run(&attach(process, pid)?, preset, apply, print)
}

#[cfg(target_os = "linux")]
fn telemetry_cmd(
    watch: &Path,
    out: &PathBuf,
    hz: f64,
    seconds: Option<f64>,
    process: &str,
    pid: Option<u32>,
    pokes: &[String],
) -> Result<()> {
    let watches = memscan::read_watch_file(watch)?;
    let p = attach(process, pid)?;
    let mut poked = memscan::apply_pokes(&p, &watches, pokes)?;
    let result = memscan::telemetry(&p, &watches, out, hz, seconds);
    poked.restore(&p, None);
    result
}

/// Index of the original's PulseAudio / PipeWire playback stream (`application.name` "Need for Speed™ Most Wanted").
#[cfg(target_os = "linux")]
fn game_audio_stream() -> Option<String> {
    let out = std::process::Command::new("pactl").args(["list", "sink-inputs"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut current, mut corked) = (None, false);
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("Sink Input #") {
            current = Some(id.trim().to_string());
        } else if line.trim() == "Corked: yes" {
            corked = true;
        } else if line.trim() == "Corked: no" {
            corked = false;
        } else if line.contains("application.name") && line.contains("Most Wanted") {
            if corked {
                eprintln!(
                    "note: the game's audio stream is paused (game unfocused or paused); it records once it plays"
                );
            }
            return current;
        }
    }
    None
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn capture_cmd(
    name: &str,
    watch: &Path,
    hz: f64,
    seconds: Option<f64>,
    no_audio: bool,
    process: &str,
    pid: Option<u32>,
    pokes: &[String],
) -> Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = PathBuf::from("ref").join(name);
    std::fs::create_dir_all(&dir)?;
    let watches = memscan::read_watch_file(watch)?;
    let p = attach(process, pid)?;
    let mut poked = memscan::apply_pokes(&p, &watches, pokes)?;
    std::fs::write(
        dir.join("meta.txt"),
        format!(
            "watch {}\nhz {hz}\nunix_time {}\npokes {}\n",
            watch.display(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            pokes.join(" ")
        ),
    )?;
    // The game's own audio stream only (not the desktop's), through parecord.
    let mut recorder = if no_audio {
        None
    } else if let Some(stream) = game_audio_stream() {
        let wav = dir.join("audio.wav");
        match std::process::Command::new("parecord")
            .arg(format!("--monitor-stream={stream}"))
            .arg("--file-format=wav")
            .arg(&wav)
            .spawn()
        {
            Ok(child) => {
                eprintln!("recording the game's audio (stream {stream}) to {}", wav.display());
                Some(child)
            }
            Err(e) => {
                eprintln!("no audio: parecord failed ({e})");
                None
            }
        }
    } else {
        eprintln!("no audio: the game's playback stream was not found (is it running with sound?)");
        None
    };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            // Only a real line stops it: a closed stdin (EOF, background runs) must not end the capture.
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
                stop.store(true, Ordering::Relaxed);
            }
        });
    }
    eprintln!("press Enter to stop");
    let result = memscan::telemetry_until(&p, &watches, &dir.join("telemetry.csv"), hz, seconds, Some(&stop));
    if let Some(mut child) = recorder.take() {
        // SIGINT lets parecord finish the WAV header.
        let _ = std::process::Command::new("kill").arg("-INT").arg(child.id().to_string()).status();
        let _ = child.wait();
    }
    poked.restore(&p, None);
    eprintln!("saved {}", dir.display());
    result
}

#[cfg(not(target_os = "linux"))]
fn capture_cmd(
    _: &str,
    _: &Path,
    _: f64,
    _: Option<f64>,
    _: bool,
    _: &str,
    _: Option<u32>,
    _: &[String],
) -> Result<()> {
    bail!("capture reads /proc/<pid>/mem and is Linux-only for now")
}

#[cfg(not(target_os = "linux"))]
fn memscan_cmd(_: &str, _: Option<u32>, _: &Path, _: bool) -> Result<()> {
    bail!("memscan reads /proc/<pid>/mem and is Linux-only for now")
}

#[cfg(not(target_os = "linux"))]
fn tune_cmd(_: &str, _: Option<u32>, _: &Path, _: bool, _: bool) -> Result<()> {
    bail!("tune writes /proc/<pid>/mem and is Linux-only for now")
}

#[cfg(not(target_os = "linux"))]
fn telemetry_cmd(_: &Path, _: &PathBuf, _: f64, _: Option<f64>, _: &str, _: Option<u32>, _: &[String]) -> Result<()> {
    bail!("telemetry reads /proc/<pid>/mem and is Linux-only for now")
}

/// `mwtool freeroam`: the free-roam collectables in TriggerData (docs/formats.md "TriggerData").
fn freeroam(store: &AssetStore, list: bool, kind: Option<&str>, all_kinds: bool) -> Result<()> {
    let data = store.trigger_data()?;
    let mut per_kind: BTreeMap<fmt::Collectable, usize> = BTreeMap::new();
    let mut per_byte: BTreeMap<(u8, &str), usize> = BTreeMap::new();
    for t in &data.triggers {
        if let Some(c) = t.collectable() {
            *per_kind.entry(c).or_default() += 1;
        }
        let shape = match t.shape {
            fmt::TriggerShape::Box { .. } => "box",
            fmt::TriggerShape::Sphere { .. } => "sphere",
            fmt::TriggerShape::Point { .. } => "point",
        };
        *per_byte.entry((t.kind, shape)).or_default() += 1;
    }
    println!(
        "TriggerData v{}: {} triggers, grid {:?} {:?}",
        data.version,
        data.triggers.len(),
        data.grid_dims,
        data.grid
    );
    for (c, n) in &per_kind {
        println!("{:<17} {n}", c.name());
    }
    if all_kinds {
        for ((k, shape), n) in &per_byte {
            println!("kind {k:3} ({:?}) {shape:<6} {n}", *k as char);
        }
    }
    if list || kind.is_some() {
        for t in &data.triggers {
            let Some(c) = t.collectable() else { continue };
            if kind.is_some_and(|k| !k.eq_ignore_ascii_case(c.name())) {
                continue;
            }
            let [x, y, z] = t.position;
            let shape = match t.shape {
                fmt::TriggerShape::Box { rotation: q, size: s } => {
                    // Heading of the box's local +z axis about Y, in degrees.
                    let yaw = (2.0 * (q[0] * q[2] + q[3] * q[1])).atan2(1.0 - 2.0 * (q[0] * q[0] + q[1] * q[1]));
                    format!("box {:.1}x{:.1}x{:.1} yaw {:.0}", s[0], s[1], s[2], yaw.to_degrees())
                }
                fmt::TriggerShape::Sphere { radius } => format!("sphere r {radius:.1}"),
                fmt::TriggerShape::Point { direction: d } => format!("point dir {:.2},{:.2},{:.2}", d[0], d[1], d[2]),
            };
            println!("{:<17} {:>8} {x:9.1} {y:7.1} {z:9.1}  flags {:#04x}  {shape}", c.name(), t.id, t.flags);
        }
    }
    Ok(())
}

/// `MW_SOUND_TRACE=<file>` with `engine-sound`: one line per driver update, `seconds rpm load normalised audible
/// gear_elapsed rpm_offset load_multiplier load_floor limiter` (as the game's `MW_SOUND_TRACE`).
fn sound_trace(t: f32, rpm: f32, s: mw_assets::engine_driver::SoundState) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file =
        FILE.get_or_init(|| std::fs::File::create(std::env::var_os("MW_SOUND_TRACE")?).ok().map(std::sync::Mutex::new));
    if let Some(f) = file {
        _ = writeln!(
            f.lock().unwrap(),
            "{t:.4} {rpm:.1} {:.4} {:.4} {:.1} {:.3} {:.4} {:.4} {:.4} {}",
            s.load,
            s.normalised,
            s.audible,
            s.gear_elapsed.min(99.0),
            s.effects.rpm_offset,
            s.effects.load_multiplier,
            s.effects.load_floor,
            u8::from(s.limiter)
        );
    }
}
