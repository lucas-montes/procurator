mod event;
mod file;
mod gateway;
mod journal_entry;
mod journal_local;

pub use event::SourceEvent;
pub use file::follow_file;
pub use gateway::follow_gateway;
pub use journal_local::follow_journalctl;
