use std::{
    fs::{File, OpenOptions},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    str::FromStr,
};

#[derive(Debug)]
enum Error {
    TunFileUnavailable(std::io::Error),
    IfaceNameInvalid(String),
    TapCreationFailed(std::io::Error),
    TapPersistenceFailed(std::io::Error),
    LinkCreationFailed(std::io::Error),
}

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

/// The netlink protocol is a socket based IPC mechanism used for communication between userspace processes and the kernel or between userspace processes themselves. The netlink protocol is based on BSD sockets and uses the AF_NETLINK address family. Every netlink protocol uses its own protocol number (e.g. NETLINK_ROUTE, NETLINK_NETFILTER, etc). Its addressing schema is based on a 32 bit port number, formerly referred to as PID, which uniquely identifies each peer.
/// <https://www.infradead.org/~tgr/libnl/doc/core.html>
/// <https://man7.org/linux/man-pages/man7/netlink.7.html>
/// <https://docs.kernel.org/userspace-api/netlink/intro.html>
struct RtNetlink {
    fd: OwnedFd,
    seq: u32,
}

impl RtNetlink {
    fn new() -> Result<Self, Error> {
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE) };
        if fd < 0 {
            return Err(Error::LinkCreationFailed(std::io::Error::last_os_error()));
        }

        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            seq: 1,
        })
    }

    /// Attach the TAP interface to a bridge. This is needed to be able to use the TAP interface for networking.
    /// <https://man7.org/linux/man-pages/man7/rtnetlink.7.html>
    fn attach_tap_interface_to_bridge(
        &self,
        iface_name: &TapName,
        bridge_name: &str,
    ) -> Result<(), Error> {
        libc::RTM_NEWLINK;

        Ok(())
    }
}

pub struct Attached(RtNetlink);

pub struct Uninitilized;

struct Tap<State = Uninitilized> {
    iface_name: TapName,
    file: File,
    state: State,
}

impl Tap {
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

        let call = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &raw mut req) };

        if call < 0 {
            return Err(Error::TapCreationFailed(std::io::Error::last_os_error()));
        }

        // Update the iface_name with the name assigned by the kernel if it was not specified.
        let iface_name = TapName(req.ifr_name);

        Ok(Self {
            iface_name,
            file,
            state: Uninitilized,
        })
    }

    /// Make the TAP interface persistent. This means that the TAP interface will not be destroyed when the file descriptor is closed. This is needed to be able to use the TAP interface for networking, since CH will re-open it by name when it starts.
    pub fn persist(self) -> Result<Self, Error> {
        // TUNSETPERSIST — keep the TAP alive after we close the fd.
        // CH will re-open it by name when it starts.
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), libc::TUNSETPERSIST, 1_i32) };
        if ret < 0 {
            return Err(Error::TapPersistenceFailed(std::io::Error::last_os_error()));
        }
        Ok(self)
    }

    /// Attach a bridge to the current opened TAP interface. This is needed to be able to use the TAP interface for networking.
    pub fn attach_bridge(self, bridge_name: &str) -> Result<Tap<Attached>, Error> {
        let link = RtNetlink::new()?;

        Ok(Tap {
            iface_name: self.iface_name,
            file: self.file,
            state: Attached(link),
        })
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
