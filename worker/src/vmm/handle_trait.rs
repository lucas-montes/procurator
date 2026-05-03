use crate::{Error, supervisor::CreateCommand};

/// Handle trait for communicating with a running VM.
pub trait Handle
where
    Self: Sized,
{
    fn ip(&self) -> &str;

    fn start(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn delete(self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn health(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn pause(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn resume(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn snapshot(
        &self,
        destination: std::path::PathBuf,
    ) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn backup_disk(
        &self,
        destination: std::path::PathBuf,
    ) -> impl Future<Output = Result<(), HandleError>> + Send;
}
