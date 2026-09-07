use futures::stream::TryStreamExt;
use rtnetlink::{LinkMessageBuilder, LinkUnspec};
use std::{
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
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
    Netlink(String),
    /// Bridge/interface not found when resolving by name.
    BridgeNotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TunFileUnavailable(e) => write!(f, "tun device open failed: {e}"),
            Error::IfaceNameInvalid(s) => write!(f, "invalid interface name: {s}"),
            Error::TapCreationFailed(e) => write!(f, "tap creation failed: {e}"),
            Error::TapPersistenceFailed(e) => write!(f, "tap persistence failed: {e}"),
            Error::Netlink(s) => write!(f, "netlink error: {s}"),
            Error::BridgeNotFound(s) => write!(f, "bridge not found: {s}"),
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
            tracing::error!(
                iface_name = %s,
                max_length = libc::IF_NAMESIZE - 1,
                "Interface name is too long",
            );
            return Err(Error::IfaceNameInvalid(s.to_string()));
        }

        let mut name = [0; libc::IF_NAMESIZE];
        for (i, c) in s.bytes().enumerate() {
            name[i] = c.cast_signed();
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
            .map(|&c| c.cast_unsigned() as char)
            .collect::<String>();
        write!(f, "{name}")
    }
}

/// Once we initialize the TAP interface we keep the fd in the structure to be able to make other operations with it.
pub struct Initialized(File);

/// Once we call `persist` the TAP interface will be kept alive by the kernel even after we drop the fd.
/// We need to persist it so cloud-hypervisor can re-open it and use it for the VM networking.
/// Otherwise, keeping the fd open would raise an error once cloud-hypervisor would try to use it.
pub struct Persisted;

pub struct Tap<State = Initialized> {
    iface_name: TapName,
    state: State,
}

impl Tap<Persisted> {
    pub async fn delete(self) -> Result<(), Error> {
        let (connection, handle, _) =
            rtnetlink::new_connection().map_err(|e| Error::Netlink(e.to_string()))?;
        tokio::spawn(connection);

        let index = handle
            .link()
            .get()
            .match_name(self.iface_name.to_string())
            .execute()
            .try_next()
            .await
            .map_err(|e| Error::Netlink(e.to_string()))?
            .ok_or_else(|| Error::BridgeNotFound(self.iface_name.to_string()))?
            .header
            .index;

        handle
            .link()
            .del(index)
            .execute()
            .await
            .map_err(|e| Error::Netlink(e.to_string()))?;

        Ok(())
    }

    /// <https://github.com/rust-netlink/rtnetlink/blob/main/examples/set_bridge_port.rs>
    pub async fn attach_to_bridge(self, bridge_name: String) -> Result<Self, Error> {
        // Establish netlink connection and find the bridge index.
        let (connection, handle, _) =
            rtnetlink::new_connection().map_err(|e| Error::Netlink(e.to_string()))?;
        tokio::spawn(connection);

        let bridge_index = handle
            .link()
            .get()
            .match_name(bridge_name.clone())
            .execute()
            .try_next()
            .await
            .map_err(|e| Error::Netlink(e.to_string()))?
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
            .map_err(|e| Error::Netlink(e.to_string()))?;

        Ok(self)
    }
}

impl<State> Tap<State> {
    pub fn name(&self) -> String {
        self.iface_name.to_string()
    }
}

impl Tap<Initialized> {
    /// Method that creaates a new TAP interface. We can optionally set a name for the interface, otherwise one will be created by the kernel.
    #[allow(clippy::cast_possible_truncation)]
    pub fn new(iface_name: Option<&str>) -> Result<Self, Error> {
        let iface_name = iface_name
            .map(TapName::from_str)
            .transpose()?
            .unwrap_or_default();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .inspect_err(|err|
            tracing::error!(%err, "Unale to open the /dev/net/tun file, which is needed to create TAP interfaces")
            )
            .map_err(Error::TunFileUnavailable)?;

        // <https://docs.kernel.org/networking/tuntap.html>
        // <https://github.com/pkts-rs/tappers/blob/master/src/linux/tap.rs>
        //
        // Flags:
        //   IFF_TAP        — Ethernet (L2) device, not point-to-point IFF_TUN.
        //   IFF_NO_PI      — no protocol info prefix on each frame.
        //   IFF_TUN_EXCL   — fail if the device already exists (don't accidentally
        //                    re-attach to a leftover from a previous run).
        //   IFF_VNET_HDR   — prepend a `struct virtio_net_hdr` to every frame.
        //                    Cloud-hypervisor negotiates virtio-net features
        //                    (TSO/UFO/checksum offloads) assuming this header is
        //                    present. Without it, packets traversing the bridge
        //                    are silently dropped on some kernel/CH combinations
        //                    (cloud-hypervisor#6550). Mirrors the working dev
        //                    launcher: `ip tuntap add ... mode tap vnet_hdr`.
        // `ifru_flags` is `c_short` (i16) on Linux but the combined flag set
        // (e.g. IFF_TUN_EXCL = 0x8000) sets the sign bit, so a checked
        // `i16::try_from` overflows. Build the mask as u16 (truncating each
        // libc constant down from c_int, exactly as a C compiler would when
        // assigning to a `short` field) and reinterpret the bit pattern as
        // i16 via `cast_signed()`. The kernel reads the field as a 16-bit
        // flag word, so the bit pattern is what matters, not the signed
        // value.
        let flags: u16 = (libc::IFF_TAP as u16)
            | (libc::IFF_NO_PI as u16)
            | (libc::IFF_TUN_EXCL as u16)
            | (libc::IFF_VNET_HDR as u16);

        let mut req = libc::ifreq {
            ifr_name: iface_name.0,
            ifr_ifru: libc::__c_anonymous_ifr_ifru {
                ifru_flags: flags.cast_signed(),
            },
        };

        let call = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &mut req) };

        if call < 0 {
            let error = std::io::Error::last_os_error();
            tracing::error!(
                %error,
                %iface_name,
                "Failed to create TAP interface with ioctl",
            );
            return Err(Error::TapCreationFailed(error));
        }

        // Update the iface_name with the name assigned by the kernel if it was not specified.
        let iface_name = TapName(req.ifr_name);

        Ok(Self {
            iface_name,
            state: Initialized(file),
        })
    }

    // Make the TAP interface persistent. This means that the TAP interface will not be destroyed when the file descriptor is closed.
    // This is needed to be able to use the TAP interface for networking, since CH will re-open it by name when it starts.
    pub fn persist(self) -> Result<Tap<Persisted>, Error> {
        // TUNSETPERSIST — keep the TAP alive after we close the fd.
        // CH will re-open it by name when it starts.
        let ret = unsafe { libc::ioctl(self.state.0.as_raw_fd(), libc::TUNSETPERSIST, 1_i32) };
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            tracing::error!(
                %error,
                iface_name = %self.iface_name,
                "Failed to make TAP interface persistent",
            );
            return Err(Error::TapPersistenceFailed(error));
        }
        Ok(Tap {
            iface_name: self.iface_name,
            state: Persisted,
        })
    }
}

// NOTE: these tests require CAP_NET_ADMIN / privileged access to create TAP
// interfaces via ioctl on /dev/net/tun. They are ignored by default so that
// `cargo test -p worker` passes in non-privileged environments (CI, local
// dev). To run them explicitly, use:
//
//   cargo test -p worker --ignored ch::tap::tests
//
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
    #[ignore = "requires CAP_NET_ADMIN to open /dev/net/tun — run with: cargo test -p worker --ignored ch::tap::tests"]
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
    #[ignore = "requires CAP_NET_ADMIN to open /dev/net/tun — run with: cargo test -p worker --ignored ch::tap::tests"]
    fn test_should_fail_because_same_name_used() {
        let tap = Tap::new(Some("tap2")).expect("Failed to create TAP interface");

        if let Err(Error::TapCreationFailed(err)) = Tap::new(Some("tap2")) {
            assert_eq!(err.kind(), std::io::ErrorKind::ResourceBusy);
            assert_eq!(err.raw_os_error(), Some(16)); // 16 is EBUSY
            assert_eq!(err.to_string(), "Device or resource busy (os error 16)");
        } else {
            panic!("Expected error when creating TAP interface with duplicate name");
        }

        assert_eq!(tap.iface_name.to_string(), "tap2");
    }
}
