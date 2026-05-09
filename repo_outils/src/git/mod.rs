mod process;
mod repo;

pub use process::{RepoError, RepoPath, clone_into_bare, create_bare_repo, delete_repo};
pub use repo::{GitRepo, RepoCache, SubmoduleInfo, filter_submodules};
