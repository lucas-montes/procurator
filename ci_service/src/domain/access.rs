//! Who is asking. Why: every read of a run or a log is checked against the
//! repo, in the application layer, whatever the caller (HTTP or in-process).

/// What a request presented to prove who sent it. Opaque here: only the
/// `Access` adapter knows what a token means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential<'a> {
    /// Nothing.
    None,
    /// A bearer token (`Authorization: Bearer …`).
    Bearer(&'a str),
}

/// Who a credential belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// Nobody in particular: no credential, or one the adapter ignores.
    Anonymous,
    /// A known user, by name.
    User(String),
}
