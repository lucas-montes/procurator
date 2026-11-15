//! Reading `nix build --log-format internal-json` stderr.
//! Why: it is the only output that says which derivation each build line
//! belongs to and which machine built it.

use std::borrow::Cow;

use serde::Deserialize;

/// Activity type of a derivation build (`actBuild`).
const ACT_BUILD: u32 = 105;
/// Result type of one build output line (`resBuildLogLine`).
const RES_BUILD_LOG_LINE: u32 = 101;
/// Message levels up to this one are shown: 0 error, 1 warning.
const MAX_SHOWN_LEVEL: u32 = 1;

/// What one stderr line says.
#[derive(Debug, PartialEq, Eq)]
pub enum NixLog<'a> {
    /// A derivation started building on `machine` (`""` = this host).
    BuildStarted {
        id: u64,
        drv: Cow<'a, str>,
        machine: Cow<'a, str>,
    },
    /// One line of the build tagged `id`.
    BuildLine { id: u64, text: Cow<'a, str> },
    /// Activity `id` ended (a build, if `id` started as one).
    Stopped { id: u64 },
    /// An error or warning, ANSI colours removed.
    Message(String),
    /// Progress and bookkeeping nobody reads.
    Ignored,
}

/// The fields of an event this module reads.
#[derive(Deserialize)]
struct Event<'a> {
    #[serde(borrow)]
    action: Cow<'a, str>,
    #[serde(default)]
    id: u64,
    #[serde(default, rename = "type")]
    kind: u32,
    #[serde(default)]
    level: u32,
    #[serde(default, borrow)]
    fields: Vec<Field<'a>>,
    #[serde(default, borrow)]
    msg: Option<Cow<'a, str>>,
}

/// An entry of an event's `fields`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Field<'a> {
    Text(#[serde(borrow)] Cow<'a, str>),
    Other(serde::de::IgnoredAny),
}

impl<'a> Field<'a> {
    fn into_text(self) -> Option<Cow<'a, str>> {
        match self {
            Self::Text(text) => Some(text),
            Self::Other(_) => None,
        }
    }
}

/// Read one stderr line. Lines that are not `@nix` events are messages.
#[must_use]
pub fn parse(line: &str) -> NixLog<'_> {
    let Some(json) = line.strip_prefix("@nix ") else {
        return if line.trim().is_empty() {
            NixLog::Ignored
        } else {
            NixLog::Message(strip_ansi(line))
        };
    };
    let Ok(event) = serde_json::from_str::<Event<'_>>(json) else {
        return NixLog::Message(strip_ansi(line));
    };
    let mut fields = event.fields.into_iter().map(Field::into_text);
    match (event.action.as_ref(), event.kind) {
        ("start", ACT_BUILD) => match (fields.next().flatten(), fields.next().flatten()) {
            (Some(drv), machine) => NixLog::BuildStarted {
                id: event.id,
                drv,
                machine: machine.unwrap_or_default(),
            },
            (None, _) => NixLog::Ignored,
        },
        ("result", RES_BUILD_LOG_LINE) => match fields.next().flatten() {
            Some(text) => NixLog::BuildLine { id: event.id, text },
            None => NixLog::Ignored,
        },
        ("stop", _) => NixLog::Stopped { id: event.id },
        ("msg", _) if event.level <= MAX_SHOWN_LEVEL => match event.msg {
            Some(msg) => NixLog::Message(strip_ansi(&msg)),
            None => NixLog::Ignored,
        },
        _ => NixLog::Ignored,
    }
}

/// Drop ANSI escape sequences (`ESC [ … final`), which Nix uses for colour.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('[') {
            // Why: parameters and intermediates are 0x20..=0x3f; the final
            // byte 0x40..=0x7e ends the sequence.
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{NixLog, parse, strip_ansi};

    #[test]
    fn build_start_names_the_derivation_and_machine() {
        let local = r#"@nix {"action":"start","fields":["/nix/store/mv5w-fail.drv","",1,1],"id":25,"level":3,"parent":0,"text":"building","type":105}"#;
        let NixLog::BuildStarted { id, drv, machine } = parse(local) else {
            panic!("not a build start");
        };
        assert_eq!(
            (id, drv.as_ref(), machine.as_ref()),
            (25, "/nix/store/mv5w-fail.drv", "")
        );

        let remote = r#"@nix {"action":"start","fields":["/nix/store/x.drv","ssh-ng://mac",1,1],"id":7,"level":3,"parent":0,"text":"","type":105}"#;
        assert!(matches!(
            parse(remote),
            NixLog::BuildStarted { machine, .. } if machine == "ssh-ng://mac"
        ));
    }

    #[test]
    fn build_lines_keep_their_text() {
        let line =
            r#"@nix {"action":"result","fields":["fail \"quoted\" line"],"id":25,"type":101}"#;
        assert_eq!(
            parse(line),
            NixLog::BuildLine {
                id: 25,
                text: "fail \"quoted\" line".into()
            }
        );
    }

    #[test]
    fn errors_are_messages_and_progress_is_ignored() {
        let error =
            r#"@nix {"action":"msg","level":0,"msg":"\u001b[31;1merror:\u001b[0m Cannot build"}"#;
        assert_eq!(
            parse(error),
            NixLog::Message(String::from("error: Cannot build"))
        );
        let info = r#"@nix {"action":"msg","level":3,"msg":"these 3 derivations will be built:"}"#;
        assert_eq!(parse(info), NixLog::Ignored);
        let progress = r#"@nix {"action":"result","fields":[101,0],"id":21,"type":106}"#;
        assert_eq!(parse(progress), NixLog::Ignored);
        let stop = r#"@nix {"action":"stop","id":25}"#;
        assert_eq!(parse(stop), NixLog::Stopped { id: 25 });
        assert_eq!(
            parse("plain text"),
            NixLog::Message(String::from("plain text"))
        );
        assert_eq!(parse("   "), NixLog::Ignored);
    }

    #[test]
    fn ansi_is_removed() {
        assert_eq!(
            strip_ansi("\u{1b}[35;1m/nix/store/x\u{1b}[0m'."),
            "/nix/store/x'."
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
