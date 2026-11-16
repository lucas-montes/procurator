//! Deploys: which runs deploy, and what they deploy.
//! Why: only the default branch's config can turn deploys on, and only for
//! its deploy branch. What is deployed is data the pusher's flake built, so
//! it is checked here before anything reaches the cluster.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::repo_config::InvalidConfig;
use super::run::GitRef;

/// The flake output a deploying run also builds: a derivation whose output
/// is the deployment file.
pub const DEPLOYMENT_ATTR: &str = "procuratorDeployment";
/// The deployment file's format version.
const VERSION: u32 = 1;
/// Largest deployment file read.
pub const MAX_DEPLOYMENT_BYTES: u64 = 1024 * 1024;
const MAX_VMS: usize = 256;
const MAX_ID: usize = 63;
const MAX_NAME: usize = 128;
const MAX_URL: usize = 2048;
const MAX_CMDLINE: usize = 4096;
const MAX_CPUS: u32 = 256;
const MIN_MEMORY_MIB: u64 = 64;
const MAX_MEMORY_MIB: u64 = 1 << 20;
const STORE: &str = "/nix/store/";
const HEADS: &str = "refs/heads/";

/// Check `deploy.branch`, also when deploys are off, so a mistake shows
/// before they are turned on.
///
/// # Errors
///
/// When it is not a plain branch name like `main` or `release/v2`.
pub fn check_branch(branch: &str) -> Result<(), InvalidConfig> {
    let plain = !branch.is_empty()
        && branch.len() <= MAX_NAME
        && !branch.starts_with('/')
        && !branch.contains("..")
        && branch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'));
    if plain {
        Ok(())
    } else {
        Err(InvalidConfig::new(format!(
            "deploy.branch {branch:?} must be a branch name like main"
        )))
    }
}

/// `deploy` from the default branch's `procurator.nix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploySettings {
    url: String,
    branch: String,
}

impl DeploySettings {
    /// Deploy runs of `branch` (e.g. `main`) to `url`.
    ///
    /// # Errors
    ///
    /// When `url` is not `http(s)://…` or `branch` is not a plain branch
    /// name.
    pub fn new(url: String, branch: String) -> Result<Self, InvalidConfig> {
        let web = url.starts_with("http://") || url.starts_with("https://");
        if !web || url.len() > MAX_URL || url.contains(char::is_whitespace) {
            return Err(InvalidConfig::new(format!(
                "deploy.url {url:?} must be an http:// or https:// URL"
            )));
        }
        check_branch(&branch)?;
        Ok(Self { url, branch })
    }

    /// Where `run.succeeded` is sent when the service deploys over HTTP.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether a run of `git_ref` deploys.
    #[must_use]
    pub fn applies_to(&self, git_ref: &GitRef) -> bool {
        git_ref.as_str().strip_prefix(HEADS) == Some(self.branch.as_str())
    }
}

/// One VM to run, as the deployment file lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeployedVm {
    id: String,
    worker: String,
    kernel: String,
    initramfs: String,
    cmdline: String,
    disk: String,
    cpus: u32,
    #[serde(rename = "memoryMiB")]
    memory_mib: u64,
}

impl DeployedVm {
    /// Stable name of the VM.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The worker it runs on.
    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    /// Kernel image store path.
    #[must_use]
    pub fn kernel(&self) -> &str {
        &self.kernel
    }

    /// Initrd store path.
    #[must_use]
    pub fn initramfs(&self) -> &str {
        &self.initramfs
    }

    /// Kernel command line (the worker appends its network settings).
    #[must_use]
    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }

    /// Raw root disk image store path (the worker boots a copy).
    #[must_use]
    pub fn disk(&self) -> &str {
        &self.disk
    }

    /// vCPUs.
    #[must_use]
    pub fn cpus(&self) -> u32 {
        self.cpus
    }

    /// Memory, in MiB.
    #[must_use]
    pub fn memory_mib(&self) -> u64 {
        self.memory_mib
    }

    fn check(&self) -> Result<(), String> {
        let id = &self.id;
        let id_ok = !id.is_empty()
            && id.len() <= MAX_ID
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-');
        if !id_ok {
            return Err(format!(
                "vm id {id:?} must be 1-{MAX_ID} of a-z 0-9 - (not starting with -)"
            ));
        }
        let worker_ok = !self.worker.is_empty()
            && self.worker.len() <= MAX_NAME
            && self
                .worker
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !worker_ok {
            return Err(format!(
                "vm {id}: worker {:?} is not a worker id",
                self.worker
            ));
        }
        for (field, path) in [
            ("kernel", &self.kernel),
            ("initramfs", &self.initramfs),
            ("disk", &self.disk),
        ] {
            if !path.starts_with(STORE) || path.contains("..") {
                return Err(format!("vm {id}: {field} {path:?} must be a store path"));
            }
        }
        if self.cmdline.trim().is_empty() || self.cmdline.len() > MAX_CMDLINE {
            return Err(format!("vm {id}: cmdline must be 1-{MAX_CMDLINE} bytes"));
        }
        if !(1..=MAX_CPUS).contains(&self.cpus) {
            return Err(format!("vm {id}: cpus must be 1-{MAX_CPUS}"));
        }
        if !(MIN_MEMORY_MIB..=MAX_MEMORY_MIB).contains(&self.memory_mib) {
            return Err(format!(
                "vm {id}: memoryMiB must be {MIN_MEMORY_MIB}-{MAX_MEMORY_MIB}"
            ));
        }
        Ok(())
    }
}

/// The deployment file's shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDto {
    version: u32,
    vms: Vec<DeployedVm>,
}

/// What a run deploys: every VM of the cluster, read from the deployment
/// file at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    path: String,
    vms: Vec<DeployedVm>,
}

impl Deployment {
    /// Read the deployment file at store path `path` from its `json`.
    ///
    /// # Errors
    ///
    /// When it is not version 1, lists no VMs or too many, repeats an id,
    /// or a VM is invalid.
    pub fn parse(path: String, json: &str) -> Result<Self, String> {
        let file: FileDto =
            serde_json::from_str(json).map_err(|e| format!("deployment file: {e}"))?;
        if file.version != VERSION {
            return Err(format!(
                "deployment file: version {} is not {VERSION}",
                file.version
            ));
        }
        if file.vms.is_empty() || file.vms.len() > MAX_VMS {
            return Err(format!("deployment file: must list 1 to {MAX_VMS} vms"));
        }
        let mut ids = HashSet::with_capacity(file.vms.len());
        for vm in &file.vms {
            vm.check().map_err(|e| format!("deployment file: {e}"))?;
            if !ids.insert(vm.id.as_str()) {
                return Err(format!("deployment file: vm id {:?} is repeated", vm.id));
            }
        }
        Ok(Self {
            path,
            vms: file.vms,
        })
    }

    /// The deployment file's store path. Why: it names the whole intent,
    /// so deploying it again changes nothing.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Every VM.
    #[must_use]
    pub fn vms(&self) -> &[DeployedVm] {
        &self.vms
    }
}

#[cfg(test)]
mod tests {
    use super::{DeploySettings, Deployment};
    use crate::domain::run::GitRef;

    fn vm(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","worker":"worker-1","kernel":"/nix/store/k/bzImage","initramfs":"/nix/store/i/initrd","cmdline":"console=ttyS0 init=/nix/store/t/init","disk":"/nix/store/d/nixos.img","cpus":2,"memoryMiB":1024}}"#
        )
    }

    fn file(vms: &[String]) -> String {
        format!(r#"{{"version":1,"vms":[{}]}}"#, vms.join(","))
    }

    #[test]
    fn a_valid_file_parses() {
        let path = String::from("/nix/store/x-procurator-deployment.json");
        let deployment = Deployment::parse(path, &file(&[vm("web"), vm("db")])).unwrap();
        assert_eq!(deployment.vms().len(), 2);
        assert_eq!(deployment.vms()[0].id(), "web");
        assert_eq!(deployment.vms()[0].memory_mib(), 1024);
        let json = serde_json::to_value(&deployment).unwrap();
        assert_eq!(json["path"], "/nix/store/x-procurator-deployment.json");
        assert_eq!(json["vms"][1]["memoryMiB"], 1024);
    }

    #[test]
    fn mistakes_are_named() {
        let bad_path = vm("web").replace("/nix/store/k/bzImage", "/boot/bzImage");
        let cases = [
            (file(&[]), "1 to 256"),
            (file(&[vm("web"), vm("web")]), "repeated"),
            (file(&[vm("Web")]), "vm id"),
            (file(&[bad_path]), "kernel"),
            (
                file(&[vm("web").replace("\"cpus\":2", "\"cpus\":0")]),
                "cpus",
            ),
            (file(&[vm("web").replace("1024", "1")]), "memoryMiB"),
            (file(&[vm("web").replace("worker-1", "a b")]), "worker"),
            (String::from(r#"{"version":2,"vms":[]}"#), "version"),
            (
                file(&[vm("web").replace("\"cpus\"", "\"cpu\"")]),
                "unknown field",
            ),
        ];
        for (json, needle) in cases {
            let error = Deployment::parse(String::new(), &json).unwrap_err();
            assert!(error.contains(needle), "{json}: {error}");
        }
    }

    #[test]
    fn only_the_deploy_branch_deploys() {
        let settings =
            DeploySettings::new(String::from("https://deploy.lan/x"), String::from("main"))
                .unwrap();
        let main = GitRef::parse(String::from("refs/heads/main")).unwrap();
        let feature = GitRef::parse(String::from("refs/heads/main-2")).unwrap();
        let tag = GitRef::parse(String::from("refs/tags/main")).unwrap();
        assert!(settings.applies_to(&main));
        assert!(!settings.applies_to(&feature));
        assert!(!settings.applies_to(&tag));
        assert!(DeploySettings::new(String::from("ftp://x"), String::from("main")).is_err());
        assert!(DeploySettings::new(String::from("http://x"), String::from("../x")).is_err());
        assert!(DeploySettings::new(String::from("http://x"), String::new()).is_err());
    }
}
