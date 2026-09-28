//! The command line (program name already stripped). Every option takes
//! `--name value` or `--name=value`.

use std::path::PathBuf;

use crate::stage::StudioRoomContext;

pub const USAGE: &str = "usage: gs-studio [--data <dir>] [--pool <dir>] [--publish <dir>]\n\
    \x20                [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]\n\
    \x20                [--room-uid <n>] [--room-title <text>] [--room-location <text>]\n\
    \x20      gs-studio [--data <dir>] --project <family>[/<creature>]:front|side|back [--grazing <facing>]\n\
    \x20                (headless: projection-first renders for a texture generator, then exit)\n\
    \x20      gs-studio [--data <dir>] --build-sheet <family>[/<creature>]\n\
    \x20                (headless: sheet.png (and normal.png) from the paintings in that folder, then exit)\n\
    \x20      gs-studio [--data <dir>] [--pool <dir>] [--publish <dir>] --save <family>[,<family>...]\n\
    \x20                (headless: the Rigs tab's Save for each -- bake, puppet, skins, publish -- then exit)\n\
    \x20      gs-studio [--data <dir>] --import-chart <scale-chart.html>\n\
    \x20                (headless: the chart's silhouettes into <data>/outlines.json, then exit)\n\
    \x20      gs-studio [--data <dir>] --export-jobs\n\
    \x20                (headless: every template and textures/jobs.json for the generator, then exit)";

/// Environment fallback for `--tab`.
pub const TAB_ENV_VAR: &str = "GS_STUDIO_TAB";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cli {
    /// `--data`: the rig data root (outlines.json, rigs/).
    pub data: Option<PathBuf>,
    /// `--pool`: the art pool / scenes directory (gs_field's base dir).
    pub pool: Option<PathBuf>,
    /// Where Save publishes baked rigs for clients to embed.
    pub publish: Option<PathBuf>,
    /// `--tab`: the tool to start on.
    pub tab: Option<String>,
    /// `--room-uid`, `--room-title`, `--room-location`: new-scene prefill,
    /// the same three a client passes to VellumFE's `studio` subcommand.
    pub room: StudioRoomContext,
    /// `--project <family>[/<creature>]:front|side|back`: write the projection-first
    /// renders (`editor::texture::export_projected`) and exit, no window.
    pub project: Option<String>,
    /// `--build-sheet <family>[/<creature>]`: project that folder's paintings
    /// (and normal maps) onto its sheet (`editor::texture::build_sheet_cli`)
    /// and exit, no window.
    pub build_sheet: Option<String>,
    /// `--save <family>[,<family>...]`: the Rigs tab's Save for each (bake,
    /// puppet, its skins and outfits into the pool, then Publish) and exit,
    /// no window -- a republish after a builder change.
    pub save: Option<String>,
    /// `--import-chart <scale-chart.html>`: write `<data>/outlines.json` from
    /// the chart (`data_root::import_outlines`, what the Rigs tab's Import
    /// does) and exit, no window.
    pub import_chart: Option<PathBuf>,
    /// `--grazing`: the facing below which projected paint counts as unpainted.
    pub grazing: Option<f32>,
    /// `--export-jobs`: write every template and `textures/jobs.json`
    /// (`editor::texture::export_jobs`) and exit, no window.
    pub export_jobs: bool,
}

/// Err carries a usage message.
pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    const OPTIONS: [&str; 12] = ["--data", "--pool", "--publish", "--tab", "--room-uid", "--room-title", "--room-location", "--project", "--grazing", "--build-sheet", "--import-chart", "--save"];
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--export-jobs" {
            cli.export_jobs = true;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (arg.as_str(), None),
        };
        if !OPTIONS.contains(&name) {
            return Err(format!("unknown argument '{arg}'\n{USAGE}"));
        }
        let value = match inline {
            Some(value) => value,
            None => it.next().ok_or_else(|| format!("{name} needs a value\n{USAGE}"))?.clone(),
        };
        match name {
            "--data" => cli.data = Some(PathBuf::from(value)),
            "--pool" => cli.pool = Some(PathBuf::from(value)),
            "--publish" => cli.publish = Some(PathBuf::from(value)),
            "--tab" => cli.tab = Some(value),
            "--room-uid" => {
                cli.room.uid = Some(value.parse().map_err(|_| format!("--room-uid needs a number, got '{value}'"))?);
            }
            "--room-title" => cli.room.title = Some(value),
            "--project" => cli.project = Some(value),
            "--build-sheet" => cli.build_sheet = Some(value),
            "--save" => cli.save = Some(value),
            "--import-chart" => cli.import_chart = Some(PathBuf::from(value)),
            "--grazing" => {
                cli.grazing = Some(value.parse().map_err(|_| format!("--grazing needs a number, got '{value}'"))?);
            }
            _ => cli.room.location = Some(value),
        }
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_both_spellings_and_rejects_junk() {
        assert_eq!(parse_args(&args(&[])), Ok(Cli::default()));
        assert_eq!(parse_args(&args(&["--data", "x/y"])).unwrap().data, Some("x/y".into()));
        assert_eq!(parse_args(&args(&["--data=z"])).unwrap().data, Some("z".into()));
        assert!(parse_args(&args(&["--data"])).is_err());
        assert!(parse_args(&args(&["--wat"])).is_err());
        assert!(parse_args(&args(&["--wat=1"])).is_err());
    }

    #[test]
    fn parses_a_headless_projection() {
        let cli = parse_args(&args(&["--data", "d", "--project", "humanoid/grim_gigas_skald:side", "--grazing=0.4"])).unwrap();
        assert_eq!(cli.project.as_deref(), Some("humanoid/grim_gigas_skald:side"));
        assert_eq!(cli.grazing, Some(0.4));
        assert!(parse_args(&args(&["--grazing", "steep"])).is_err());
        assert!(parse_args(&args(&["--export-jobs", "--data", "d"])).unwrap().export_jobs);
        assert_eq!(parse_args(&args(&["--build-sheet", "troll/cave_troll"])).unwrap().build_sheet.as_deref(), Some("troll/cave_troll"));
        assert_eq!(parse_args(&args(&["--save", "humanoid,troll"])).unwrap().save.as_deref(), Some("humanoid,troll"));
        assert_eq!(parse_args(&args(&["--import-chart", "x/scale-chart.html"])).unwrap().import_chart, Some(PathBuf::from("x/scale-chart.html")));
    }

    #[test]
    fn parses_pool_tab_and_room_context() {
        let cli = parse_args(&args(&[
            "--pool=C:/art",
            "--tab",
            "preview",
            "--room-uid",
            "47009",
            "--room-title=Kitchen Garden",
            "--room-location",
            "Castle Anwyn",
            "--data",
            "d",
        ]))
        .unwrap();
        assert_eq!(cli.pool, Some("C:/art".into()));
        assert_eq!(cli.data, Some("d".into()));
        assert_eq!(cli.tab.as_deref(), Some("preview"));
        assert_eq!(
            cli.room,
            StudioRoomContext {
                uid: Some(47009),
                title: Some("Kitchen Garden".into()),
                location: Some("Castle Anwyn".into())
            }
        );
        assert!(parse_args(&args(&["--room-uid", "kitchen"])).is_err());
    }
}
