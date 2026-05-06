mod process;
mod repo;

pub use process::{
    create_bare_repo,
clone_into_bare,
delete_repo,
RepoPath,
RepoError
};
pub use repo::{GitRepo, SubmoduleInfo, filter_submodules};
