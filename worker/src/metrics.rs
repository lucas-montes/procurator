//! Host and process resource sampling from `/proc`, written in the worker
//! state lines (see `context/specs/log-events.md`). Every reader returns
//! `None`/zero on platforms or processes where the data is missing, so a
//! failed sample never breaks a heartbeat.

/// Host-level sample.
#[derive(Debug)]
pub struct HostStats {
    mem_total: u64,
    mem_available: u64,
    load1: f64,
    cpus: u64,
}

impl HostStats {
    pub fn mem_total(&self) -> u64 {
        self.mem_total
    }

    pub fn mem_available(&self) -> u64 {
        self.mem_available
    }

    pub fn load1(&self) -> f64 {
        self.load1
    }

    pub fn cpus(&self) -> u64 {
        self.cpus
    }
}

/// Samples memory (`/proc/meminfo`), load (`/proc/loadavg`) and CPU count.
pub async fn host() -> HostStats {
    let meminfo = tokio::fs::read_to_string("/proc/meminfo")
        .await
        .unwrap_or_default();
    let loadavg = tokio::fs::read_to_string("/proc/loadavg")
        .await
        .unwrap_or_default();
    HostStats {
        mem_total: meminfo_kib(&meminfo, "MemTotal:").saturating_mul(1024),
        mem_available: meminfo_kib(&meminfo, "MemAvailable:").saturating_mul(1024),
        load1: loadavg
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.0),
        cpus: std::thread::available_parallelism()
            .ok()
            .and_then(|count| u64::try_from(count.get()).ok())
            .unwrap_or(0),
    }
}

/// Cumulative user + system CPU time of `pid`, in milliseconds.
pub async fn process_cpu_ms(pid: u32) -> u64 {
    let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
        .await
        .unwrap_or_default();
    cpu_ms_from_stat(&stat, clock_ticks_per_second())
}

/// Resident memory of `pid`, in bytes.
pub async fn process_rss_bytes(pid: u32) -> u64 {
    let status = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
        .await
        .unwrap_or_default();
    meminfo_kib(&status, "VmRSS:").saturating_mul(1024)
}

fn clock_ticks_per_second() -> u64 {
    // SAFETY: `sysconf` has no preconditions; it returns -1 on error.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks)
        .ok()
        .filter(|ticks| *ticks > 0)
        .unwrap_or(100)
}

/// Parses `utime` + `stime` (fields 14 and 15) of `/proc/<pid>/stat`. The
/// command name (field 2) may contain spaces, so fields are counted after
/// its closing parenthesis.
fn cpu_ms_from_stat(stat: &str, ticks_per_second: u64) -> u64 {
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return 0;
    };
    let mut fields = rest.split_whitespace().skip(11);
    let utime: u64 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let stime: u64 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    utime.saturating_add(stime).saturating_mul(1000) / ticks_per_second.max(1)
}

/// Reads a `Key:   <n> kB` line from `/proc/meminfo` or `/proc/<pid>/status`.
fn meminfo_kib(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Sums the byte counters of a Cloud Hypervisor `vm.counters` response:
/// `(net_rx, net_tx, disk_read, disk_write)` across all devices.
pub fn sum_counters(counters: &serde_json::Value) -> (u64, u64, u64, u64) {
    let mut totals = (0_u64, 0_u64, 0_u64, 0_u64);
    let Some(devices) = counters.as_object() else {
        return totals;
    };
    for device in devices.values() {
        let read = |name: &str| {
            device
                .get(name)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        totals.0 = totals.0.saturating_add(read("rx_bytes"));
        totals.1 = totals.1.saturating_add(read("tx_bytes"));
        totals.2 = totals.2.saturating_add(read("read_bytes"));
        totals.3 = totals.3.saturating_add(read("write_bytes"));
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::{cpu_ms_from_stat, meminfo_kib, sum_counters};

    #[test]
    fn parses_cpu_time_after_command_name_with_spaces() {
        let stat = "4242 (cloud hyper) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 3 0 12345 1000 200";
        assert_eq!(cpu_ms_from_stat(stat, 100), 3000);
    }

    #[test]
    fn reads_kib_lines() {
        let text = "MemTotal:       16000 kB\nMemAvailable:    8000 kB\n";
        assert_eq!(meminfo_kib(text, "MemAvailable:"), 8000);
        assert_eq!(meminfo_kib(text, "VmRSS:"), 0);
    }

    #[test]
    fn sums_device_counters() {
        let counters = serde_json::json!({
            "_net2": {"rx_bytes": 10, "tx_bytes": 20, "rx_frames": 1},
            "_disk0": {"read_bytes": 300, "write_bytes": 400},
            "_disk1": {"read_bytes": 5}
        });
        assert_eq!(sum_counters(&counters), (10, 20, 305, 400));
    }
}
