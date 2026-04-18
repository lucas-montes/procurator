use futures::stream::TryStreamExt;
use rtnetlink::{LinkMessageBuilder, LinkUnspec};
use std::{
    fs::{File, OpenOptions},
    os::fd::{AsRawFd},
    str::FromStr,
};

#[derive(Debug)]
pub enum Error {
    TunFileUnavailable(std::io::Error),
    IfaceNameInvalid(String),
    TapCreationFailed(std::io::Error),
    TapPersistenceFailed(std::io::Error),
    /// Errors coming from rtnetlink or other netlink operations. Stored as a string
    /// to avoid depending on rtnetlink's error types in the error API.
    NetlinkError(String),
    /// Bridge/interface not found when resolving by name.
    BridgeNotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TunFileUnavailable(e) => write!(f, "tun device open failed: {}", e),
            Error::IfaceNameInvalid(s) => write!(f, "invalid interface name: {}", s),
            Error::TapCreationFailed(e) => write!(f, "tap creation failed: {}", e),
            Error::TapPersistenceFailed(e) => write!(f, "tap persistence failed: {}", e),
            Error::NetlinkError(s) => write!(f, "netlink error: {}", s),
            Error::BridgeNotFound(s) => write!(f, "bridge not found: {}", s),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
struct TapName([libc::c_char; libc::IF_NAMESIZE]);

impl Default for TapName {
    fn default() -> Self {
        Self([0; libc::IF_NAMESIZE])
    }
}

impl FromStr for TapName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > libc::IF_NAMESIZE - 1 {
            return Err(Error::IfaceNameInvalid(s.to_string()));
        }

        let mut name = [0; libc::IF_NAMESIZE];
        for (i, c) in s.bytes().enumerate() {
            name[i] = c as libc::c_char;
        }

        Ok(Self(name))
    }
}

impl std::fmt::Display for TapName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self
            .0
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8 as char)
            .collect::<String>();
        write!(f, "{}", name)
    }
}

pub struct Tap {
    iface_name: TapName,
    _file: File,
}

impl Tap {
    pub fn name(&self) -> String {
        self.iface_name.to_string()
    }

    /// Method that creaates a new TAP interface. We can optionally set a name for the interface, otherwise one will be created by the kernel.
    pub fn new(iface_name: Option<&str>) -> Result<Self, Error> {
        let iface_name = iface_name
            .map(TapName::from_str)
            .transpose()?
            .unwrap_or_default();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .map_err(Error::TunFileUnavailable)?;

        // <https://docs.kernel.org/networking/tuntap.html>
        // <https://github.com/pkts-rs/tappers/blob/master/src/linux/tap.rs>
        // The flags and fd needed to be sent to ioctl to create the tap interface.
        // I used the links above to understand how it works.
        let flags = libc::IFF_TAP | libc::IFF_NO_PI | libc::IFF_TUN_EXCL;

        let mut req = libc::ifreq {
            ifr_name: iface_name.0,
            ifr_ifru: libc::__c_anonymous_ifr_ifru {
                ifru_flags: flags as i16,
            },
        };

        let call = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &mut req) };

        if call < 0 {
            return Err(Error::TapCreationFailed(std::io::Error::last_os_error()));
        }

        // Update the iface_name with the name assigned by the kernel if it was not specified.
        let iface_name = TapName(req.ifr_name);

        Ok(Self { iface_name, _file: file })
    }

    /// Make the TAP interface persistent. This means that the TAP interface will not be destroyed when the file descriptor is closed. This is needed to be able to use the TAP interface for networking, since CH will re-open it by name when it starts.
    ///NOTE: if we keep the structure alive we don't need to make it persistent and the clean up of the TAP device can be done with rust's Drop trait
    fn persist(self) -> Result<Self, Error> {
        // TUNSETPERSIST — keep the TAP alive after we close the fd.
        // CH will re-open it by name when it starts.
        let ret = unsafe { libc::ioctl(self._file.as_raw_fd(), libc::TUNSETPERSIST, 1_i32) };
        if ret < 0 {
            return Err(Error::TapPersistenceFailed(std::io::Error::last_os_error()));
        }
        Ok(self)
    }

    /// <https://github.com/rust-netlink/rtnetlink/blob/main/examples/set_bridge_port.rs>
    pub async fn attach_to_bridge(self, bridge_name: String) -> Result<Self, Error> {
        // Establish netlink connection and find the bridge index.
        let (connection, handle, _) =
            rtnetlink::new_connection().map_err(|e| Error::NetlinkError(e.to_string()))?;
        tokio::spawn(connection);

        let bridge_index = handle
            .link()
            .get()
            .match_name(bridge_name.clone())
            .execute()
            .try_next()
            .await
            .map_err(|e| Error::NetlinkError(e.to_string()))?
            .ok_or(Error::BridgeNotFound(bridge_name))
            .map(|l| l.header.index)?;

        handle
            .link()
            .set(
                LinkMessageBuilder::<LinkUnspec>::default()
                    .name(self.iface_name.to_string())
                    .controller(bridge_index)
                    .up()
                    .build(),
            )
            .execute()
            .await
            .map_err(|e| Error::NetlinkError(e.to_string()))?;

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{Error, Tap, TapName};

    #[test]
    fn test_tap_name() {
        let name_str = "tap0";
        let tap_name = TapName::from_str(name_str).unwrap();
        assert_eq!(tap_name.to_string(), name_str);

        let default = TapName::default();
        assert_eq!(default.to_string(), "");
        assert_eq!(default.0, [0; libc::IF_NAMESIZE]);
    }

    #[test]
    fn test_create_tap() {
        let tap1 = Tap::new(Some("tap1")).expect("Failed to create TAP interface with name");

        let tap0 = Tap::new(None).expect("Failed to create TAP interface without name");
        assert!(!tap0.iface_name.to_string().is_empty());
        // The second tap should have a different name assigned by the kernel.
        // It seems that the kernel uses an incremental number for the name, so we can expect it to be "tap0" if "tap1" was created first and other tests uses tapX where x > 0.
        assert_eq!(tap0.iface_name.to_string(), "tap0");

        assert_eq!(tap1.iface_name.to_string(), "tap1");
    }

    #[test]
    fn test_should_fail_because_same_name_used() {
        let tap = Tap::new(Some("tap2")).expect("Failed to create TAP interface");

        if let Err(Error::TapCreationFailed(err)) = Tap::new(Some("tap2")) {
            assert_eq!(err.kind(), std::io::ErrorKind::ResourceBusy);
            assert_eq!(err.raw_os_error(), Some(16)); // 16 is EBUSY
            assert_eq!(err.to_string(), "Device or resource busy (os error 16)");
        } else {
            panic!("Expected error when creating TAP interface with duplicate name");
        };

        assert_eq!(tap.iface_name.to_string(), "tap2");
    }
}
