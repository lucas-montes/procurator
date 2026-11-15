//! Journal export JSON entries (`journalctl -o json`, journal-gatewayd with
//! `Accept: application/json`): one object per line. The service's own log
//! line is the `MESSAGE` field.

use std::borrow::Cow;

use serde::Deserialize;

#[derive(Deserialize)]
struct Entry<'a> {
    #[serde(rename = "MESSAGE", borrow, default)]
    message: Option<Message<'a>>,
    #[serde(rename = "__CURSOR", borrow, default)]
    cursor: Option<Cow<'a, str>>,
}

/// journald exports non-UTF-8 or control-character messages as byte arrays.
#[derive(Deserialize)]
#[serde(untagged)]
enum Message<'a> {
    Text(#[serde(borrow)] Cow<'a, str>),
    Bytes(Vec<u8>),
}

/// Parsed entry: the message text and the entry cursor.
pub struct JournalLine {
    message: Option<String>,
    cursor: Option<String>,
}

impl JournalLine {
    pub fn into_parts(self) -> (Option<String>, Option<String>) {
        (self.message, self.cursor)
    }
}

/// Parses one exported entry; `None` when the line isn't an entry.
pub fn parse(line: &str) -> Option<JournalLine> {
    let entry: Entry<'_> = serde_json::from_str(line).ok()?;
    let message = entry.message.map(|message| match message {
        Message::Text(text) => text.into_owned(),
        Message::Bytes(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
    });
    Some(JournalLine {
        message,
        cursor: entry.cursor.map(Cow::into_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_message_and_cursor() {
        let (message, cursor) =
            parse(r#"{"__CURSOR":"s=abc;i=1","MESSAGE":"{\"log.event\":\"msg\"}","_PID":"1"}"#)
                .expect("entry")
                .into_parts();
        assert_eq!(message.as_deref(), Some(r#"{"log.event":"msg"}"#));
        assert_eq!(cursor.as_deref(), Some("s=abc;i=1"));
    }

    #[test]
    fn decodes_byte_array_messages() {
        let (message, _) = parse(r#"{"MESSAGE":[104,105]}"#)
            .expect("entry")
            .into_parts();
        assert_eq!(message.as_deref(), Some("hi"));
    }
}
