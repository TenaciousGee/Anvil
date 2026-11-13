// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! `anvil` command-line front end.
//!
//! Shares the exact planning code the GUI uses, so it doubles as a headless
//! batch tool and a way to inspect what a job will do before running it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anvil_core::crop::Crop;
use anvil_core::jobfile::{tool_versions, JobFile};
use anvil_core::model::SourceInfo;
use anvil_core::pipeline::{default_output_path, default_work_dir, plan_job, JobPlan, PlanOptions};
use anvil_core::preset::Preset;
use anvil_core::preset::{Origin, PresetStore, DEFAULT_PRESET};
use anvil_core::probe::{parse_ffprobe_json, probe_file};
use anvil_core::progress::{format_duration, Progress};
use anvil_core::runner::{clean_intermediates, run_plan, CancelToken, Event};
use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "anvil",
    version,
    about = "Plan and inspect Anvil encodes from the command line"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe a media file and summarise what Anvil sees.
    Probe {
        /// Media file, or a saved ffprobe JSON file with --from-json.
        input: PathBuf,
        /// Treat INPUT as saved ffprobe JSON instead of running ffprobe.
        #[arg(long)]
        from_json: bool,
        /// Print the parsed source as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the full job plan (steps and commands) for a file.
    Plan {
        input: PathBuf,
        #[arg(long)]
        from_json: bool,
        /// Preset id (see `anvil presets`).
        #[arg(long, short, default_value = DEFAULT_PRESET)]
        preset: String,
        /// Output folder.
        #[arg(long, short, default_value = ".")]
        out: PathBuf,
        /// Working folder for intermediate files.
        #[arg(long)]
        work: Option<PathBuf>,
        /// Manual crop as LEFT,RIGHT,TOP,BOTTOM.
        #[arg(long, value_parser = parse_crop, conflicts_with = "autocrop")]
        crop: Option<Crop>,
        /// Detect the crop with FFmpeg before planning.
        #[arg(long)]
        autocrop: bool,
        #[command(flatten)]
        edits: Edits,
        /// Print the plan as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Detect black borders with FFmpeg cropdetect.
    Autocrop {
        input: PathBuf,
        #[arg(long, default_value_t = 12)]
        samples: u32,
    },
    /// Write a bash script that runs the whole job (encode today, before the
    /// built-in runner exists).
    Script {
        input: PathBuf,
        #[arg(long, short, default_value = DEFAULT_PRESET)]
        preset: String,
        #[arg(long, short, default_value = ".")]
        out: PathBuf,
        #[arg(long, value_parser = parse_crop, conflicts_with = "autocrop")]
        crop: Option<Crop>,
        #[arg(long)]
        autocrop: bool,
        #[command(flatten)]
        edits: Edits,
        /// Where to write the script (default: next to the output).
        #[arg(long)]
        script: Option<PathBuf>,
    },
    /// Encode a file: plan it, save the job file, run every step with live
    /// progress, then verify the result against the plan.
    Run {
        /// Media file to encode (or use --job).
        #[arg(required_unless_present = "job")]
        input: Option<PathBuf>,
        /// Re-run a saved job file instead of planning a new job.
        #[arg(long, conflicts_with_all = ["input", "preset", "crop", "autocrop"])]
        job: Option<PathBuf>,
        #[arg(long, short)]
        preset: Option<String>,
        /// Output folder.
        #[arg(long, short, default_value = ".")]
        out: PathBuf,
        /// Working folder for intermediate files (default: OUT/.anvil-work/<name>).
        #[arg(long)]
        work: Option<PathBuf>,
        #[arg(long, value_parser = parse_crop, conflicts_with = "autocrop")]
        crop: Option<Crop>,
        #[arg(long)]
        autocrop: bool,
        #[command(flatten)]
        edits: Edits,
        /// Keep intermediate files (encoded video and audio) after success.
        #[arg(long)]
        keep_work: bool,
        /// Plan and save the job file, but do not run it.
        #[arg(long)]
        dry_run: bool,
    },
    /// List a Blu-ray or DVD folder's titles; --copy copies the main title
    /// to one MKV (no re-encoding), ready for `anvil run`.
    Disc {
        /// The disc folder (the one holding BDMV or VIDEO_TS).
        path: PathBuf,
        #[arg(long)]
        copy: bool,
        /// Copy this title instead of the main one (an id from the list).
        #[arg(long, requires = "copy")]
        title: Option<String>,
        /// Folder for the copy (default: beside the disc folder).
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// List a file's chapters, or generate them, and optionally export them
    /// as Matroska XML (.xml) or OGM text (.txt).
    Chapters {
        input: PathBuf,
        /// Generate a chapter every MINUTES instead of reading the file's.
        #[arg(long, value_name = "MINUTES")]
        every: Option<f64>,
        /// Write the chapters here (.xml for Matroska XML, else OGM).
        #[arg(long, value_name = "FILE")]
        export: Option<PathBuf>,
    },
    /// Turn image subtitles (PGS .sup or VobSub .idx) into SRT text with
    /// Tesseract.
    Ocr {
        input: PathBuf,
        /// Subtitle language (eng, fre, de...); picks Tesseract's language data.
        #[arg(long, default_value = "eng")]
        lang: String,
        /// Only the captions marked forced.
        #[arg(long)]
        forced_only: bool,
        /// Where to write the SRT (default: beside the input).
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Check which encoding tools are installed.
    Tools,
    /// Show an encoder's option page with a preset's values, or (--check)
    /// compare the option pages with the installed tools' --help.
    Options {
        /// x265, x264, svt_av1, rav1e, nvencc, qsvencc, vceencc, ffmpeg_vaapi
        /// (all with --check when left out).
        encoder: Option<String>,
        #[arg(long, short, default_value = "quality-default")]
        preset: String,
        #[arg(long)]
        check: bool,
    },
    /// Check this machine's encoders end to end: make a 10-second HDR10 +
    /// Dolby Vision 8.1 test film with letterbox bars and encode it with
    /// each usable GPU preset (and Quality Default with --cpu), cropping and
    /// verifying everything Anvil checks, Dolby Vision L5 included.
    Selftest {
        /// Folder for the test film, the encodes and the report.
        #[arg(long, short, default_value = "anvil-selftest")]
        out: PathBuf,
        /// Also test the CPU encoder (x265, Quality Default).
        #[arg(long)]
        cpu: bool,
        /// Only these presets (comma separated), usable or not.
        #[arg(long, value_delimiter = ',')]
        presets: Vec<String>,
    },
    /// Time a few short encodes with a preset and estimate how long the
    /// whole job will take on this machine.
    Estimate {
        input: PathBuf,
        #[arg(long, short, default_value = DEFAULT_PRESET)]
        preset: String,
        /// Output folder (the test encodes use its work folder).
        #[arg(long, short, default_value = ".")]
        out: PathBuf,
        #[arg(long, value_parser = parse_crop, conflicts_with = "autocrop")]
        crop: Option<Crop>,
        #[arg(long)]
        autocrop: bool,
        #[command(flatten)]
        edits: Edits,
    },
    /// List presets, or print one fully resolved.
    Presets {
        /// Preset id to show resolved as TOML.
        #[arg(long)]
        show: Option<String>,
    },
}

/// Per-job changes to tracks and chapters (the same edits the GUI makes).
#[derive(clap::Args, Debug, Default, Clone)]
struct Edits {
    /// Output audio track, in order (repeat for each track; the first is the
    /// default). N[:TARGET[:KBPS[:CHANNELS]]][,norm][,delay=MS][,title=TEXT]
    /// with TARGET copy, ac3, eac3, aac, opus, flac or dts_core.
    /// Example: --audio 1 --audio 1:ac3:640 --audio 2:opus:160:2,norm
    #[arg(long = "audio", value_name = "SPEC")]
    audio: Vec<String>,
    /// Output subtitle track, in order (repeat for each). N for source track
    /// N (1 = first) or a file (.srt .ass .ssa .sup .idx), then options
    /// ,forced ,forced-only ,default ,lang=XX ,title=TEXT.
    /// Example: --sub 1 --sub 1,forced-only,title=Forced --sub film.en.srt
    #[arg(long = "sub", value_name = "SPEC", conflicts_with = "no_subs")]
    subs: Vec<String>,
    /// Leave out every subtitle track.
    #[arg(long)]
    no_subs: bool,
    /// Chapters: keep (the source's), none, every=MINUTES, or a Matroska XML
    /// or OGM chapter file to use instead.
    #[arg(long, value_name = "WHAT")]
    chapters: Option<String>,
}

impl Edits {
    fn chapter_plan(
        &self,
        src: &SourceInfo,
    ) -> AnyResult<Option<anvil_core::chapters::ChapterPlan>> {
        use anvil_core::chapters::{self, ChapterPlan};
        let Some(what) = self.chapters.as_deref() else {
            return Ok(None);
        };
        Ok(Some(match what {
            "keep" => ChapterPlan::Keep,
            "none" => ChapterPlan::Drop,
            _ => {
                let mut list = if let Some(m) = what.strip_prefix("every=") {
                    let minutes: f64 = m
                        .parse()
                        .map_err(|_| format!("'{m}' is not a number of minutes"))?;
                    chapters::every(minutes, src.duration_secs.unwrap_or(0.0))?
                } else {
                    chapters::parse(&std::fs::read_to_string(what)?)?
                };
                chapters::tidy(&mut list, src.duration_secs);
                ChapterPlan::Custom { chapters: list }
            }
        }))
    }
}

fn parse_crop(s: &str) -> Result<Crop, String> {
    let v: Vec<u32> = s
        .split(',')
        .map(|p| p.trim().parse::<u32>().map_err(|e| format!("'{p}': {e}")))
        .collect::<Result<_, _>>()?;
    match v.as_slice() {
        [l, r, t, b] => Ok(Crop {
            left: *l,
            right: *r,
            top: *t,
            bottom: *b,
        }),
        _ => Err("expected LEFT,RIGHT,TOP,BOTTOM".into()),
    }
}

fn main() -> ExitCode {
    anvil_core::paths::prepare_vapoursynth_env();
    // Presets and the queue from when Anvil was called Reelforge.
    let home = std::env::var("HOME").ok();
    for dir in anvil_core::paths::migrate_from_reelforge(home.as_deref()) {
        eprintln!("Moved your Reelforge settings to {}", dir.display());
    }
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

fn run(cli: Cli) -> AnyResult<()> {
    match cli.cmd {
        Cmd::Ocr {
            input,
            lang,
            forced_only,
            out,
        } => {
            let out = out.unwrap_or_else(|| input.with_extension("srt"));
            let work = std::env::temp_dir().join(format!("anvil-ocr-{}", std::process::id()));
            let (srt, n) = anvil_core::ocr::file_to_srt(
                &input,
                Some(&lang),
                forced_only,
                &work,
                &|| false,
                |p| eprint!("\r  reading {:.0}%", p * 100.0),
            )?;
            eprintln!();
            let _ = std::fs::remove_dir_all(&work);
            std::fs::write(&out, srt)?;
            println!("Read {n} captions into {}", out.display());
        }
        Cmd::Chapters {
            input,
            every,
            export,
        } => {
            use anvil_core::chapters;
            let src = load_source(&input, false)?;
            let list = match every {
                Some(m) => chapters::every(m, src.duration_secs.unwrap_or(0.0))?,
                None => src.chapters.clone(),
            };
            if list.is_empty() {
                println!("No chapters (use --every MINUTES to make some).");
            }
            for c in &list {
                println!("{}  {}", chapters::format_time(c.start_secs), c.title);
            }
            if let Some(path) = export {
                let xml = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
                let text = if xml {
                    chapters::to_matroska_xml(&list, "eng")
                } else {
                    chapters::to_ogm(&list)
                };
                std::fs::write(&path, text)?;
                println!("Wrote {}", path.display());
            }
        }
        Cmd::Disc {
            path,
            copy,
            title,
            out,
        } => {
            use anvil_core::disc;
            let d = disc::scan(&path, Duration::from_secs(60))?;
            let main = disc::main_title(&d).ok_or("no titles")?;
            let chosen = match &title {
                Some(id) => d
                    .titles
                    .iter()
                    .find(|t| &t.id == id)
                    .ok_or_else(|| format!("no title {id} on this disc"))?,
                None => main,
            };
            println!("{} {} ({} titles):", d.kind.name(), d.label, d.titles.len());
            for t in &d.titles {
                let mark = if t.source == main.source { "*" } else { " " };
                println!("  {mark} {}", t.describe(d.kind));
            }
            println!("  * main title");
            if copy {
                let dir =
                    out.unwrap_or_else(|| d.root.parent().unwrap_or(Path::new(".")).to_path_buf());
                let target = disc::remux_path(&d, chosen, &dir);
                for n in chosen.notes(d.kind) {
                    println!("! {n}");
                }
                let cancel = std::sync::atomic::AtomicBool::new(false);
                // Print every 10%: start below zero so 0..9% shows once.
                let mut last = -10i64;
                disc::copy_title(chosen, &target, &cancel, |p| {
                    let pct = (p * 100.0) as i64;
                    if pct / 10 != last / 10 {
                        eprintln!("  copying {pct}%");
                        last = pct;
                    }
                })?;
                println!("Copied to {}", target.display());
            }
        }
        Cmd::Probe {
            input,
            from_json,
            json,
        } => {
            let src = load_source(&input, from_json)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&src)?);
            } else {
                print_summary(&src);
            }
        }
        Cmd::Plan {
            input,
            from_json,
            preset,
            out,
            work,
            crop,
            autocrop,
            edits,
            json,
        } => {
            let src = load_source(&input, from_json)?;
            let (preset, plan) = prepare(
                &src, &preset, &out, work, crop, autocrop, !from_json, &edits,
            )?;

            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
                return Ok(());
            }
            println!("Preset:  {}", preset.name);
            println!("Source:  {}", plan.source);
            println!("Output:  {}", plan.output.display());
            println!(
                "Picture: {}x{} (crop {:?})",
                plan.output_size.0, plan.output_size.1, plan.crop
            );
            println!("Badges:  {}", plan.badges.join(" | "));
            println!("\nAudio (output order):");
            for (i, a) in plan.audio.iter().enumerate() {
                println!(
                    "  {}. {}{}",
                    i + 1,
                    a.describe(),
                    if a.default { "  [default]" } else { "" }
                );
            }
            println!("\nSubtitles (output order):");
            for (i, s) in plan.subtitles.iter().enumerate() {
                let flags = [(s.default, "default"), (s.forced, "forced")]
                    .iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, f)| *f)
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  {}. {}{}",
                    i + 1,
                    s.describe(),
                    if flags.is_empty() {
                        String::new()
                    } else {
                        format!("  [{flags}]")
                    }
                );
            }
            if !plan.warnings.is_empty() {
                println!("\nWarnings:");
                for w in &plan.warnings {
                    println!("  ! {w}");
                }
            }
            println!("\nSteps:");
            for (i, s) in plan.steps.iter().enumerate() {
                let deps = if s.depends_on.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  (after {})",
                        s.depends_on
                            .iter()
                            .map(|d| (d + 1).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                println!("  {}. {}{}", i + 1, s.label, deps);
                if let Some(c) = &s.command {
                    println!("     $ {}", c.to_shell());
                }
                if let Some((path, contents)) = &s.file {
                    println!(
                        "     -> {} ({} lines)",
                        path.display(),
                        contents.lines().count()
                    );
                }
            }
        }
        Cmd::Autocrop { input, samples } => {
            let src = load_source(&input, false)?;
            let crop = detect_crop(&src, samples)?;
            let (w, h) = crop.validate(src.video.width, src.video.height)?;
            println!(
                "{},{},{},{}  -> {w}x{h}",
                crop.left, crop.right, crop.top, crop.bottom
            );
        }
        Cmd::Script {
            input,
            preset,
            out,
            crop,
            autocrop,
            edits,
            script,
        } => {
            let src = load_source(&input, false)?;
            let (_, plan) = prepare(&src, &preset, &out, None, crop, autocrop, true, &edits)?;
            let path = script.unwrap_or_else(|| plan.output.with_extension("sh"));
            write_script(&path, &anvil_core::pipeline::to_shell_script(&plan))?;
            for w in &plan.warnings {
                println!("! {w}");
            }
            println!(
                "Wrote {}\nRun it with: bash {}",
                path.display(),
                anvil_core::command::shell_quote(&path.to_string_lossy())
            );
        }
        Cmd::Run {
            input,
            job,
            preset,
            out,
            work,
            crop,
            autocrop,
            edits,
            keep_work,
            dry_run,
        } => {
            let job = match job {
                Some(path) => {
                    let job = JobFile::load(&path)?;
                    println!(
                        "Loaded {} (preset {}, created by Anvil {})",
                        path.display(),
                        job.preset.name,
                        job.anvil_version
                    );
                    job
                }
                None => {
                    let input = input.expect("clap requires input without --job");
                    let src = load_source(&input, false)?;
                    let preset_id = preset.unwrap_or_else(|| DEFAULT_PRESET.to_string());
                    let (resolved, plan) =
                        prepare(&src, &preset_id, &out, work, crop, autocrop, true, &edits)?;
                    let job = JobFile::new(
                        &preset_id,
                        resolved,
                        plan,
                        tool_versions(Duration::from_secs(10)),
                    );
                    job.save(&job.plan.work_dir.join("job.toml"))?;
                    job
                }
            };
            print_plan_header(&job.preset, &job.plan);
            println!("Job file: {}", job.plan.work_dir.join("job.toml").display());
            if dry_run {
                return Ok(());
            }
            execute(&job.plan, keep_work)?;
        }
        Cmd::Tools => {
            let mut missing_required = false;
            for t in anvil_core::tools::detect_all(Duration::from_secs(10)) {
                let mark = if t.found() {
                    "ok "
                } else if t.required {
                    "MISSING"
                } else {
                    "-  "
                };
                missing_required |= t.required && !t.found();
                println!(
                    "{mark:<8} {:<15} {:<45} {}",
                    t.name,
                    t.version.clone().unwrap_or_else(|| if t.found() {
                        "(version unknown)".into()
                    } else {
                        "not installed".into()
                    }),
                    t.purpose
                );
            }
            println!("\nHardware encoders:");
            for r in anvil_core::hw::detect("/dev/dri/renderD128", Duration::from_secs(15)) {
                println!(
                    "{:<8} {:<15} {}",
                    if r.usable { "ok " } else { "-  " },
                    r.kind.label(),
                    r.summary()
                );
            }
            if missing_required {
                println!("\nSome required tools are missing. On CachyOS/Arch: ./install-cachyos.sh --with-tools --with-aur");
            }
        }
        Cmd::Selftest { out, cpu, presets } => selftest(&out, cpu, presets)?,
        Cmd::Estimate {
            input,
            preset,
            out,
            crop,
            autocrop,
            edits,
        } => {
            let src = load_source(&input, false)?;
            let (preset, plan) = prepare(&src, &preset, &out, None, crop, autocrop, true, &edits)?;
            let probe = plan.speed_probe.ok_or(
                "cannot estimate: the video is copied, or its length or frame rate is unknown",
            )?;
            eprintln!(
                "Timing {} short {} encodes with {} ...",
                probe.samples.len(),
                probe.encoder,
                preset.name
            );
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let e = anvil_core::estimate::run(&probe, &cancel, |d, n| eprintln!("  {d}/{n}"))?
                .ok_or("cancelled")?;
            println!("{}", e.describe());
        }
        Cmd::Options {
            encoder,
            preset,
            check,
        } => {
            use anvil_core::encoder::EncoderKind;
            use anvil_core::schema;
            let kinds: Vec<EncoderKind> = match &encoder {
                Some(e) => vec![toml::Value::String(e.clone())
                    .try_into()
                    .map_err(|_| format!("unknown encoder '{e}'"))?],
                None if check => EncoderKind::ALL.to_vec(),
                None => return Err("name an encoder, or use --check for all".into()),
            };
            let mut problems = 0;
            for kind in kinds {
                let s = schema::schema(kind);
                if check {
                    if anvil_core::tools::which(&s.program).is_none() {
                        println!(
                            "-  {:<13} {} not installed, skipped",
                            kind.label(),
                            s.program
                        );
                        continue;
                    }
                    let out = anvil_core::process::run_with_timeout(
                        &anvil_core::tools::program(&s.program),
                        &s.help_args,
                        Duration::from_secs(20),
                    )?;
                    let missing =
                        schema::missing_from_help(&s, &format!("{}\n{}", out.stdout, out.stderr));
                    if missing.is_empty() {
                        println!(
                            "ok {:<13} all {} options found in {} {}",
                            kind.label(),
                            s.options.len(),
                            s.program,
                            s.help_args.join(" ")
                        );
                    } else {
                        problems += 1;
                        println!(
                            "!! {:<13} not in this version's help: {}",
                            kind.label(),
                            missing.join(" ")
                        );
                    }
                    continue;
                }
                let p = preset_store()?.resolve(&preset, None)?;
                let v = toml::Value::try_from(&p.video)?;
                let args: Vec<String> = v
                    .get(schema::preset_key(kind))
                    .and_then(|t| t.get("extra_args"))
                    .cloned()
                    .and_then(|a| a.try_into().ok())
                    .unwrap_or_default();
                let (rows, other) = schema::page_rows(&s, &args);
                let mut group = String::new();
                for r in rows {
                    if r[2] != group {
                        group = r[2].clone();
                        println!("\n{group}");
                    }
                    let value = if r[4].is_empty() {
                        format!("({})", r[5])
                    } else {
                        r[4].clone()
                    };
                    println!("  {:<28} {:<16} {}", r[1], value, r[0]);
                }
                println!(
                    "\nOther arguments: {}",
                    if other.is_empty() { "none" } else { &other }
                );
                println!(
                    "Edit on the Presets page, or in the preset's [video.{}] extra_args.",
                    schema::preset_key(kind)
                );
            }
            if problems > 0 {
                return Err(format!(
                    "{problems} option page(s) need updating for the installed tools"
                )
                .into());
            }
        }
        Cmd::Presets { show } => {
            let store = preset_store()?;
            match show {
                Some(id) => {
                    print!("{}", toml::to_string_pretty(&store.resolve(&id, None)?)?);
                    let rules = store.rules(&id)?;
                    if !rules.is_empty() {
                        println!("\n# Rules (applied per source, in order):");
                        for r in rules {
                            let mut what: Vec<String> = Vec::new();
                            if !r.set.is_empty() {
                                what.push(anvil_core::preset::flatten(&r.set));
                            }
                            if !r.add_filters.is_empty() {
                                what.push(format!("adds {}", r.add_filters.join(", ")));
                            }
                            println!(
                                "#   {}: when {} -> {}",
                                r.name,
                                r.when.describe(),
                                what.join("; ")
                            );
                        }
                    }
                }
                None => {
                    let settings = anvil_core::settings::Settings::load();
                    let mut hidden = 0;
                    for (id, origin) in store.ids() {
                        let p = store.resolve(id, None)?;
                        let tag = if origin == Origin::User {
                            "user"
                        } else {
                            "built-in"
                        };
                        match anvil_core::settings::encoder_available(p.video.encoder, &settings) {
                            Ok(()) => println!("{id:<18} {:<9} {}  {}", tag, p.name, p.description),
                            Err(why) => {
                                hidden += 1;
                                println!(
                                    "{id:<18} {:<9} {}  (not usable here: {why})",
                                    tag, p.name
                                );
                            }
                        }
                    }
                    if hidden > 0 && !settings.hardware_encoders {
                        println!("\nGPU encoders are off (CPU only): set hardware_encoders = true in ~/.config/anvil/settings.toml, or use the Tools page.");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resolve the preset, detect the crop, map track IDs for non-MKV sources
/// and build the plan.
#[allow(clippy::too_many_arguments)]
fn prepare(
    src: &SourceInfo,
    preset_id: &str,
    out: &Path,
    work: Option<PathBuf>,
    crop: Option<Crop>,
    autocrop: bool,
    real_file: bool,
    edits: &Edits,
) -> AnyResult<(Preset, JobPlan)> {
    let resolved = preset_store()?.resolve_for(preset_id, src, None)?;
    for rule in &resolved.applied {
        eprintln!("Preset rule applied: {rule}");
    }
    let preset = resolved.preset;
    let crop = if autocrop {
        Some(detect_crop(src, 12)?)
    } else {
        crop
    };
    let track_ids = if real_file {
        match anvil_core::trackmap::identify(src, Duration::from_secs(120)) {
            Ok(map) => map,
            Err(e) => {
                eprintln!(
                    "! Could not map track IDs with mkvmerge ({e}); assuming Matroska numbering."
                );
                None
            }
        }
    } else {
        None
    };
    let source_path = Path::new(&src.path);
    let plan = plan_job(
        src,
        &preset,
        &PlanOptions {
            work_dir: work.unwrap_or_else(|| default_work_dir(source_path, out)),
            output: default_output_path(source_path, out),
            crop,
            audio_override: if edits.audio.is_empty() {
                None
            } else {
                Some(anvil_core::audio::plan_from_specs(
                    &edits.audio,
                    &src.audio,
                )?)
            },
            chapters: edits.chapter_plan(src)?,
            filters: None,
            subtitle_override: if edits.no_subs {
                Some(Vec::new())
            } else if edits.subs.is_empty() {
                None
            } else {
                let mut plan = Vec::new();
                for spec in &edits.subs {
                    anvil_core::subtitles::parse_sub_spec(spec, &src.subtitles, &mut plan)?;
                }
                Some(plan)
            },
            track_ids,
        },
    )?;
    Ok((preset, plan))
}

fn print_plan_header(preset: &Preset, plan: &JobPlan) {
    println!("Source:  {}", plan.source);
    println!("Output:  {}", plan.output.display());
    println!("Preset:  {}", preset.name);
    if !plan.badges.is_empty() {
        println!("Keeps:   {}", plan.badges.join(" · "));
    }
    for w in &plan.warnings {
        println!("  ! {w}");
    }
}

/// Run a plan in the terminal with a live progress line.
/// Make the self-test film: 1920x1080 with 140-pixel bars, 10 s at 24 fps,
/// x265 HDR10 with a generated Dolby Vision 8.1 RPU (L5 = the bars), and an
/// AC-3 track.
fn make_test_film(dir: &std::path::Path) -> AnyResult<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let film = dir.join("anvil-selftest-source.mkv");
    if film.exists() {
        return Ok(film);
    }
    std::fs::write(
        dir.join("rpu.json"),
        r#"{"cm_version": "V40", "length": 240,
 "level5": {"active_area_left_offset": 0, "active_area_right_offset": 0, "active_area_top_offset": 140, "active_area_bottom_offset": 140},
 "level6": {"max_display_mastering_luminance": 1000, "min_display_mastering_luminance": 1, "max_content_light_level": 1000, "max_frame_average_light_level": 400}}"#,
    )?;
    let script = r#"set -e
ffmpeg -v error -y -f lavfi -i "testsrc2=size=1920x800:rate=24,pad=1920:1080:0:140:black" -t 10 \
  -pix_fmt yuv420p10le -strict -1 -f yuv4mpegpipe - |
  x265 --log-level error --input - --y4m --preset ultrafast --crf 20 --output-depth 10 --profile main10 \
    --colorprim bt2020 --transfer smpte2084 --colormatrix bt2020nc --range limited --hdr10 --repeat-headers \
    --master-display "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)" --max-cll 1000,400 \
    --output base.hevc
dovi_tool generate -j rpu.json -o RPU.bin >/dev/null
dovi_tool inject-rpu -i base.hevc --rpu-in RPU.bin -o dv.hevc >/dev/null
ffmpeg -v error -y -f lavfi -i "sine=frequency=440:sample_rate=48000" -t 10 -ac 6 -c:a ac3 -b:a 448k audio.ac3
mkvmerge -q -o anvil-selftest-source.mkv --default-duration 0:24fps dv.hevc audio.ac3
rm -f base.hevc dv.hevc RPU.bin audio.ac3
"#;
    eprintln!("Making the test film (HDR10 + Dolby Vision 8.1, letterboxed) ...");
    let status = std::process::Command::new("sh")
        .args(["-c", script])
        .current_dir(dir)
        .status()?;
    if !status.success() {
        return Err(
            "could not make the test film (needs ffmpeg, x265, dovi_tool and mkvmerge)".into(),
        );
    }
    Ok(film)
}

fn selftest(out: &std::path::Path, cpu: bool, only: Vec<String>) -> AnyResult<()> {
    use anvil_core::settings::{encoder_available, Settings};
    let film = make_test_film(out)?;
    let store = preset_store()?;
    let settings = Settings::load();
    let mut ids: Vec<String> = if only.is_empty() {
        ["fast-nvenc", "fast-qsv", "fast-amd", "fast-vaapi"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        only.clone()
    };
    if cpu && only.is_empty() {
        ids.push(DEFAULT_PRESET.to_string());
    }
    let src = load_source(&film, false)?;
    let mut report = vec![format!(
        "Anvil {} self-test, {}",
        env!("CARGO_PKG_VERSION"),
        film.display()
    )];
    let mut failed = 0;
    for id in ids {
        let p = store.resolve(&id, None)?;
        if only.is_empty() {
            if let Err(why) = encoder_available(p.video.encoder, &settings) {
                report.push(format!("-     {:<16} skipped: {why}", p.name));
                continue;
            }
        }
        println!("\n==== {} ({}) ====", p.name, p.video.encoder.label());
        let dir = out.join(&id);
        let started = std::time::Instant::now();
        let result = prepare(&src, &id, &dir, None, None, true, true, &Edits::default())
            .and_then(|(_, plan)| execute(&plan, false).map(|_| plan));
        let secs = started.elapsed().as_secs_f64();
        match result {
            Ok(plan) => report.push(format!(
                "PASS  {:<16} {:.0} s, {} -> {}",
                p.name,
                secs,
                plan.badges.join(" · "),
                plan.output.display()
            )),
            Err(e) => {
                failed += 1;
                // The error's first lines name the failed checks.
                let text = e.to_string();
                let lines: Vec<&str> = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .take(4)
                    .collect();
                report.push(format!("FAIL  {:<16} {}", p.name, lines.join(" | ")));
            }
        }
    }
    let text = report.join("\n") + "\n";
    std::fs::write(out.join("selftest-report.txt"), &text)?;
    println!(
        "\n{text}Report: {}",
        out.join("selftest-report.txt").display()
    );
    if failed > 0 {
        return Err(format!(
            "{failed} preset(s) failed; logs are in each preset's .anvil-work folder"
        )
        .into());
    }
    Ok(())
}

fn execute(plan: &JobPlan, keep_work: bool) -> AnyResult<()> {
    use std::io::{IsTerminal, Write};
    // One Ctrl+C handler for the whole process (selftest runs several jobs).
    static CANCEL: std::sync::OnceLock<CancelToken> = std::sync::OnceLock::new();
    let cancel = CANCEL
        .get_or_init(|| {
            let token = CancelToken::new();
            let c = token.clone();
            let _ = ctrlc::set_handler(move || c.cancel());
            token
        })
        .clone();

    let tty = std::io::stdout().is_terminal();
    let total = plan.steps.len();
    let mut current: Option<(usize, Progress)> = None;
    let mut last_plain = std::time::Instant::now();
    let clear = |tty: bool| {
        if tty {
            print!("\r\x1b[2K");
        }
    };
    println!();
    let result = run_plan(plan, &cancel, |event| match event {
        Event::StepStarted { index, label } => {
            clear(tty);
            println!("▶ [{}/{}] {}", index + 1, total, label);
        }
        Event::StepProgress { index, progress } => {
            let line = format!("    {} {}", plan.steps[index].label, progress.summary());
            if tty {
                print!("\r\x1b[2K{line}");
                let _ = std::io::stdout().flush();
            } else if last_plain.elapsed() >= Duration::from_secs(30) {
                println!("{line}");
                last_plain = std::time::Instant::now();
            }
            current = Some((index, progress));
        }
        Event::StepFinished {
            index,
            label,
            ok,
            secs,
        } => {
            clear(tty);
            println!(
                "{} [{}/{}] {} ({})",
                if ok { "✔" } else { "✘" },
                index + 1,
                total,
                label,
                format_duration(secs.round() as u64)
            );
            if current.as_ref().is_some_and(|(i, _)| *i == index) {
                current = None;
            }
        }
        Event::Verified { report } => {
            clear(tty);
            println!("\nVerification:");
            for c in &report.checks {
                let mark = if c.ok {
                    "✔"
                } else if c.critical {
                    "✘"
                } else {
                    "!"
                };
                println!("  {mark} {:<38} {}", c.name, c.detail);
            }
            println!();
        }
    });
    clear(tty);
    match result {
        Ok(report) => {
            if !keep_work {
                let freed = clean_intermediates(plan);
                if freed > 0 {
                    let size = if freed >= 1_000_000_000 {
                        format!("{:.1} GB", freed as f64 / 1e9)
                    } else {
                        format!("{:.0} MB", freed as f64 / 1e6)
                    };
                    println!(
                        "Removed intermediate files ({size}). Logs and job file kept in {}",
                        plan.work_dir.display()
                    );
                }
            }
            println!(
                "Done in {}: {}",
                format_duration(report.secs.round() as u64),
                plan.output.display()
            );
            Ok(())
        }
        Err(anvil_core::Error::Cancelled) => {
            Err("cancelled; partial files left in the work folder".into())
        }
        Err(e) => Err(e.into()),
    }
}

fn load_source(input: &Path, from_json: bool) -> AnyResult<SourceInfo> {
    if from_json {
        let mut src = parse_ffprobe_json(&std::fs::read_to_string(input)?)?;
        if src.path.is_empty() {
            src.path = input.to_string_lossy().into_owned();
        }
        return Ok(src);
    }
    let home = std::env::var("HOME").ok();
    let path = anvil_core::paths::normalise_input_path(&input.to_string_lossy(), home.as_deref());
    if !Path::new(&path).exists() {
        return Err(format!("file not found: {path}").into());
    }
    Ok(probe_file(&path, Duration::from_secs(120))?)
}

fn write_script(path: &Path, contents: &str) -> AnyResult<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn detect_crop(src: &SourceInfo, samples: u32) -> AnyResult<Crop> {
    let (crop, measured) =
        anvil_core::crop::detect(src, samples, Duration::from_secs(60), |_, _| {});
    if measured == 0 {
        return Err("auto-crop could not measure any frames (is FFmpeg installed?)".into());
    }
    Ok(crop)
}

fn preset_store() -> AnyResult<PresetStore> {
    let mut store = PresetStore::builtin()?;
    if let Some(dir) = user_preset_dir() {
        store.load_user_dir(&dir)?;
    }
    Ok(store)
}

fn user_preset_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("anvil").join("presets"))
}

fn print_summary(src: &SourceInfo) {
    let v = &src.video;
    println!("{}", src.path);
    if let Some(d) = src.duration_secs {
        println!(
            "  Duration  {}:{:02}:{:02}",
            (d / 3600.0) as u32,
            ((d % 3600.0) / 60.0) as u32,
            (d % 60.0) as u32
        );
    }
    println!(
        "  Video     {} {}x{} {}-bit {:?}{}{}{}",
        v.codec,
        v.width,
        v.height,
        v.bit_depth,
        v.dynamic_range,
        v.dolby_vision
            .as_ref()
            .map(|d| format!(", Dolby Vision profile {}", d.profile))
            .unwrap_or_default(),
        if v.hdr10_plus { ", HDR10+" } else { "" },
        if v.interlaced { ", interlaced" } else { "" },
    );
    if let Some(fr) = v.frame_rate {
        println!(
            "  Frames    {:.3} fps{}",
            fr.as_f64(),
            if v.vfr_suspected {
                " (variable frame rate suspected)"
            } else {
                ""
            }
        );
    }
    for a in &src.audio {
        println!(
            "  Audio #{:<2} {} {}",
            a.index,
            a.label(),
            if a.codec.is_lossless() {
                "lossless"
            } else {
                ""
            }
        );
    }
    for s in &src.subtitles {
        println!(
            "  Subs  #{:<2} {:?} {}{}",
            s.index,
            s.codec,
            s.language.as_deref().unwrap_or("und"),
            if s.forced { " (forced)" } else { "" }
        );
    }
    println!("  Chapters  {}", src.chapter_count);
}
