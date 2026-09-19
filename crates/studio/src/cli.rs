//! The command line (program name already stripped). Every option takes
//! `--name value` or `--name=value`.

use std::path::PathBuf;

use crate::stage::StudioRoomContext;

pub const USAGE: &str = "usage: gs-studio [--data <dir>] [--pool <dir>] [--publish <dir>]\n\
    \x20                [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]\n\
    \x20                [--room-uid <n>] [--room-title <text>] [--room-location <text>]";

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
}

/// Err carries a usage message.
pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    const OPTIONS: [&str; 7] = ["--data", "--pool", "--publish", "--tab", "--room-uid", "--room-title", "--room-location"];
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
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
