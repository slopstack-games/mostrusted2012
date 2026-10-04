use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

mod crack;
mod damage;
#[cfg(target_os = "linux")]
mod memscan;
mod preview;
mod survey;

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
        #[arg(long = "style", value_enum, default_values_t = [crack::Style::Snake, crack::Style::TitleSnake, crack::Style::Camel])]
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
    },
    /// Count world triangles per shader (all track units), to prioritise ports.
    ShaderUsage,
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
    /// Disassemble the compiled D3D11 programs (DXBC) a shader uses.
    ShaderAsm {
        /// Exact shader name, e.g. `DriveableSurface`.
        name: String,
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
        Cmd::Telemetry { watch, out, hz, seconds, process, pid } => {
            return telemetry_cmd(watch, out, *hz, *seconds, process, *pid);
        }
        _ => {}
    }
    let game = GameDir::locate(cli.game_dir.as_deref())?;
    match cli.cmd {
        Cmd::Memscan { .. } | Cmd::Telemetry { .. } => unreachable!("handled before locating the game"),
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
        Cmd::ShaderAsm { name } => {
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
        Cmd::EngineSound { car, out, trace, legacy, dry } => {
            let store = AssetStore::new(game)?;
            let v = store.find_vehicle(&car)?.with_context(|| format!("no car matches {car}"))?;
            let b = store.bundle(v.hi_bundle.as_deref().context("no bundle")?).context("bundle")?;
            let h = mw_assets::handling::VehicleHandling::from_bundle(&store, &b).unwrap_or_default();
            let (idle, redline) = (h.idle_rpm.unwrap_or(900.0), h.redline_rpm.unwrap_or(7000.0));
            // A recorded trace (`seconds rpm throttle [gear]`, stepwise as the game updates it), or a 4 s
            // rev-up on full throttle and a 4 s coast down, in physics RPM.
            let points: Vec<[f32; 5]> = match &trace {
                Some(p) => std::fs::read_to_string(p)?
                    .lines()
                    .filter_map(|l| {
                        let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                        (v.len() >= 3).then(|| {
                            [v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0), v.get(4).copied().unwrap_or(0.0)]
                        })
                    })
                    .collect(),
                None => vec![],
            };
            let duration = points.last().map_or(8.0, |p| p[0]);
            let input_at = |t: f32, k: &mut usize| -> [f32; 4] {
                if !points.is_empty() {
                    while *k + 1 < points.len() && points[*k + 1][0] <= t {
                        *k += 1;
                    }
                    [points[*k][1], points[*k][2], points[*k][3], points[*k][4]]
                } else if t < 4.0 {
                    [idle + (redline - idle) * t / 4.0, 1.0, 1.0, 0.0]
                } else {
                    [redline - (redline - idle) * (t - 4.0) / 4.0, 0.0, 1.0, 0.0]
                }
            };
            let voice = if legacy {
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
                        "engine bank: {} sequences, {} channels, rpm axis {:.0}-{:.0}, dominant order {}, physics {idle:.0}-{redline:.0} rpm",
                        eb.bank.sequences.len(),
                        eb.bank.channels.len(),
                        eb.rpm_range.0,
                        eb.rpm_range.1,
                        eb.harmonic_order
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
                    let mut player = mw_audio::sequence::Player::new(eb.bank.clone(), params);
                    player.dry = dry;
                    player.enable_meter();
                    // The game updates the engine at its physics rate; 64 Hz here.
                    let tick = rate / 64;
                    for i in 0..(rate as f32 * duration) as usize {
                        if i % tick == 0 {
                            let t = i as f32 / rate as f32;
                            let [rpm, throttle, gear, speed] = input_at(t, &mut k);
                            let input = mw_assets::engine_driver::EngineInput {
                                rpm,
                                throttle,
                                gear: gear as i32,
                                speed_mph: speed.abs() / 0.447_04,
                                idle_rpm: idle,
                                redline_rpm: redline,
                            };
                            driver.update(tick as f32 / rate as f32, &input);
                        }
                        samples.push((player.next_sample() * 32767.0).clamp(-32768.0, 32767.0) as i16);
                    }
                    if std::env::var_os("MW_AUDIO_METER").is_some() {
                        let n = samples.len().max(1) as f64;
                        for (id, e) in player.meter() {
                            println!("  channel {id:016x}: {:.1} dB", 10.0 * (e / n + 1e-12).log10());
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
                        let [rpm, load, _, _] = input_at(t, &mut k);
                        samples.push((mixer.next_sample(rpm, load) * 32767.0).clamp(-32768.0, 32767.0) as i16);
                    }
                    let pcm = mw_audio::Pcm { channels: 1, sample_rate: rate as u32, samples };
                    std::fs::write(&out, pcm.to_wav())?;
                }
            }
            println!("{}: {:.1} s -> {}", v.name, duration, out.display());
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
fn telemetry_cmd(
    watch: &Path,
    out: &PathBuf,
    hz: f64,
    seconds: Option<f64>,
    process: &str,
    pid: Option<u32>,
) -> Result<()> {
    let watches = memscan::read_watch_file(watch)?;
    memscan::telemetry(&attach(process, pid)?, &watches, out, hz, seconds)
}

#[cfg(not(target_os = "linux"))]
fn memscan_cmd(_: &str, _: Option<u32>, _: &Path, _: bool) -> Result<()> {
    bail!("memscan reads /proc/<pid>/mem and is Linux-only for now")
}

#[cfg(not(target_os = "linux"))]
fn telemetry_cmd(_: &Path, _: &PathBuf, _: f64, _: Option<f64>, _: &str, _: Option<u32>) -> Result<()> {
    bail!("telemetry reads /proc/<pid>/mem and is Linux-only for now")
}
