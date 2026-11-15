//! `journalctl -f -o json -u <unit>` on the console host.

use std::{process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::Sender,
    time::sleep,
};

use super::{
    event::{SourceEvent, line, status},
    journal_entry,
};

/// Follows the unit's journal forever, restarting `journalctl` when it
/// exits. The first run reads back `entries` lines; restarts resume after
/// the last cursor seen.
pub async fn follow_journalctl(source: usize, unit: String, entries: u64, tx: Sender<SourceEvent>) {
    let mut cursor: Option<String> = None;
    loop {
        let mut command = Command::new("journalctl");
        command
            .args(["--follow", "--output=json", "--no-pager", "--unit"])
            .arg(&unit)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        match &cursor {
            Some(cursor) => command.arg(format!("--after-cursor={cursor}")),
            None => command.arg(format!("--lines={entries}")),
        };
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                status(&tx, source, Some(format!("cannot run journalctl: {err}"))).await;
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let Some(stdout) = child.stdout.take() else {
            status(&tx, source, Some("journalctl has no stdout".to_owned())).await;
            sleep(Duration::from_secs(5)).await;
            continue;
        };
        status(&tx, source, None).await;
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(text)) = lines.next_line().await {
            let Some(entry) = journal_entry::parse(&text) else {
                continue;
            };
            let (message, entry_cursor) = entry.into_parts();
            if entry_cursor.is_some() {
                cursor = entry_cursor;
            }
            if let Some(message) = message
                && !line(&tx, source, message).await
            {
                return;
            }
        }
        let _ = child.wait().await;
        status(&tx, source, Some("journalctl exited".to_owned())).await;
        sleep(Duration::from_secs(2)).await;
    }
}
