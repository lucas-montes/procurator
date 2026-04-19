use std::net::Ipv4Addr;

use tracing::error;

use crate::database::Database;

/// The network identity assigned to a VM.
#[derive(Debug, Clone)]
pub struct IpLease {
    ip: String,
    mask: String,
    mac: String,
}

impl IpLease {
    fn new(ip: Ipv4Addr, mask: Ipv4Addr) -> Self {
        Self {
            mac: mac_from_ip(ip),
            ip: ip.to_string(),
            mask: mask.to_string(),
        }
    }

    pub fn ip(&self) -> &str {
        &self.ip
    }

    pub fn mask(&self) -> &str {
        &self.mask
    }

    pub fn mac(&self) -> &str {
        &self.mac
    }
}

/// Allocates static IPs by incrementing from the last reserved address.
/// Leases are persisted in the worker database via [`Database`].
#[derive(Debug, Clone)]
pub struct IpAllocator {
    db: Database,
    start: Ipv4Addr,
    end: Ipv4Addr,
    mask: Ipv4Addr,
}

impl IpAllocator {
    pub fn new(db: Database, start: Ipv4Addr, end: Ipv4Addr, mask: Ipv4Addr) -> Self {
        Self { db, start, end, mask }
    }

    /// Reserve the next available IP for `vm_id` and persist it.
    pub async fn reserve(&self, vm_id: &str) -> Result<IpLease, sqlx::Error> {
        let next = self.next_ip().await?;
        let mac = mac_from_ip(next);
        self.db.reserve_ip(vm_id, next, &mac).await?;
        Ok(IpLease::new(next, self.mask))
    }

    /// Release the IP lease held by `vm_id`.
    pub async fn release(&self, vm_id: &str) -> Result<(), sqlx::Error> {
        self.db.release_ip(vm_id).await
    }

    async fn next_ip(&self) -> Result<Ipv4Addr, sqlx::Error> {
        // Prefer reusing a released slot over allocating a fresh one.
        if let Some(free) = self.db.first_free_ip().await? {
            return Ok(free);
        }

        // No free slots — increment past the highest ever-allocated IP.
        let candidate = match self.db.last_reserved_ip().await? {
            None => self.start,
            Some(last) => Ipv4Addr::from(u32::from(last) + 1),
        };

        if u32::from(candidate) > u32::from(self.end) {
            error!(%candidate, end = %self.end, "Failed to reserve IP: pool exhausted");
            return Err(sqlx::Error::Protocol("IP pool exhausted".into()));
        }

        Ok(candidate)
    }
}

/// Derive a locally-administered MAC from an IPv4 address.
/// Uses the `02:xx:xx:xx:xx:xx` prefix (locally administered, unicast).
fn mac_from_ip(ip: Ipv4Addr) -> String {
    let octets = ip.octets();
    format!(
        "02:00:{:02x}:{:02x}:{:02x}:{:02x}",
        octets[0], octets[1], octets[2], octets[3]
    )
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::IpAllocator;
    use crate::database::Database;

    #[tokio::test]
    async fn reserve_reuses_free_slots_before_incrementing() {
        let db = Database::new("sqlite::memory:").await;
        let allocator = IpAllocator::new(
            db,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 255, 254),
            Ipv4Addr::new(255, 255, 0, 0),
        );

        let lease1 = allocator.reserve("vm-aaa").await.expect("first reserve");
        assert_eq!(lease1.ip(), "10.0.0.1");
        assert_eq!(lease1.mask(), "255.255.0.0");
        assert!(lease1.mac().starts_with("02:"));

        let lease2 = allocator.reserve("vm-bbb").await.expect("second reserve");
        assert_eq!(lease2.ip(), "10.0.0.2");

        // Release the first slot — it becomes free for reuse.
        allocator.release("vm-aaa").await.expect("release");

        // Next allocation reuses the free slot instead of incrementing.
        let lease3 = allocator.reserve("vm-ccc").await.expect("third reserve");
        assert_eq!(lease3.ip(), "10.0.0.1");

        // A further allocation increments past the highest ever-used IP.
        let lease4 = allocator.reserve("vm-ddd").await.expect("fourth reserve");
        assert_eq!(lease4.ip(), "10.0.0.3");
    }

    #[tokio::test]
    async fn pool_exhausted_returns_error() {
        let db = Database::new("sqlite::memory:").await;
        let allocator = IpAllocator::new(
            db,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 1),  // only one IP
            Ipv4Addr::new(255, 255, 0, 0),
        );

        allocator.reserve("vm-aaa").await.expect("first reserve should succeed");
        let err = allocator.reserve("vm-bbb").await;
        assert!(err.is_err());
    }
}
