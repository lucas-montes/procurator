//! Which systems the CI host and its remote builders can build, read from
//! `nix config show --json`.
//! Why: the host's Nix config is the one place builders are declared (it is
//! what the daemon dispatches with), so the CI service never keeps its own
//! list that could drift.

use std::borrow::Cow;

use serde::Deserialize;

/// The settings this module reads. Each is `{"value": …}`.
#[derive(Deserialize)]
struct Config<'a> {
    #[serde(borrow)]
    system: Setting<Cow<'a, str>>,
    #[serde(rename = "extra-platforms")]
    extra_platforms: Setting<Vec<String>>,
    #[serde(borrow)]
    builders: Setting<Cow<'a, str>>,
    #[serde(rename = "max-jobs")]
    max_jobs: Setting<u64>,
}

#[derive(Deserialize)]
struct Setting<T> {
    value: T,
}

/// Parse the config. `read` loads a machines file for `builders = @<path>`.
///
/// # Errors
///
/// When the JSON lacks a setting or `read` fails.
pub fn parse(
    json: &str,
    read: impl Fn(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let config: Config<'_> = serde_json::from_str(json).map_err(|e| format!("nix config: {e}"))?;
    let local = config.system.value.as_ref();
    let mut systems = Vec::new();
    // Why: `max-jobs = 0` means the host never builds itself.
    if config.max_jobs.value > 0 {
        systems.push(String::from(local));
        systems.extend(config.extra_platforms.value);
    }
    let builders = config.builders.value;
    let machines = match builders.strip_prefix('@') {
        Some(path) => Cow::Owned(read(path.trim())?),
        None => builders,
    };
    for spec in machines.split([';', '\n']) {
        systems.extend(machine_systems(spec, local));
    }
    Ok(systems)
}

/// The systems of one machine line: field 2, comma-separated; `-` or missing
/// means the host's own system. Blank and `#` lines have none.
fn machine_systems<'a>(spec: &'a str, local: &'a str) -> impl Iterator<Item = String> + 'a {
    let spec = spec.split('#').next().unwrap_or_default();
    let mut fields = spec.split_whitespace();
    let present = fields.next().is_some();
    let listed = match fields.next() {
        Some("-") | None => local,
        Some(listed) => listed,
    };
    listed
        .split(',')
        .filter(move |system| present && !system.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn config(builders: &str, max_jobs: u64) -> String {
        format!(
            r#"{{"system":{{"value":"x86_64-linux"}},"extra-platforms":{{"value":["i686-linux"]}},"builders":{{"value":{builders:?}}},"max-jobs":{{"value":{max_jobs}}},"other":{{"value":1}}}}"#
        )
    }

    fn no_file(path: &str) -> Result<String, String> {
        Err(format!("unexpected read of {path}"))
    }

    #[test]
    fn host_only() {
        assert_eq!(
            parse(&config("", 8), no_file).unwrap(),
            ["x86_64-linux", "i686-linux"]
        );
    }

    #[test]
    fn inline_builders_add_their_systems() {
        let builders = "ssh-ng://mac aarch64-darwin,x86_64-darwin /key 4 ; ssh://arm aarch64-linux";
        assert_eq!(
            parse(&config(builders, 8), no_file).unwrap(),
            [
                "x86_64-linux",
                "i686-linux",
                "aarch64-darwin",
                "x86_64-darwin",
                "aarch64-linux"
            ]
        );
    }

    #[test]
    fn machines_file_and_defaults() {
        let read = |path: &str| {
            assert_eq!(path, "/etc/nix/machines");
            Ok(String::from(
                "# comment\n\nssh://same\nssh://dash - /key\nssh://pi aarch64-linux # trailing\n",
            ))
        };
        assert_eq!(
            parse(&config("@/etc/nix/machines", 0), read).unwrap(),
            ["x86_64-linux", "x86_64-linux", "aarch64-linux"],
            "max-jobs 0 drops the host, a machine without systems is the host's system"
        );
    }

    #[test]
    fn the_file_nixos_writes_for_remote_builders() {
        // Why: verbatim what services.procurator.ci.remoteBuilders renders.
        let read = |_: &str| {
            Ok(String::from(
                "ssh-ng://nix-builder@mac.lab aarch64-darwin,x86_64-darwin /run/secrets/k 4 1 big-parallel - -\n",
            ))
        };
        assert_eq!(
            parse(&config("@/etc/nix/machines", 8), read).unwrap(),
            [
                "x86_64-linux",
                "i686-linux",
                "aarch64-darwin",
                "x86_64-darwin"
            ]
        );
    }

    #[test]
    fn unreadable_machines_file_is_an_error() {
        assert!(parse(&config("@/nope", 1), no_file).is_err());
    }
}
