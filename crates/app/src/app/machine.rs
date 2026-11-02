//! What the machine is: board, processor, memory, kernel and local address.
//!
//! The values come from `/proc` and `/sys`, with a UDP socket used only to ask
//! the kernel which source address it would use. That sends no packet, but it
//! does belong to the shell rather than to `cread-core`, which stays free of
//! network code.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;

pub(super) struct Machine {
    model: Option<String>,
    cpu: Option<String>,
    cores: usize,
    memory: Option<u64>,
    kernel: Option<String>,
    system: Option<String>,
    arch: &'static str,
    ip: Option<IpAddr>,
}

impl Machine {
    pub(super) fn read() -> Self {
        let cpuinfo = read_text("/proc/cpuinfo").unwrap_or_default();
        let model = read_text("/sys/firmware/devicetree/base/model")
            .or_else(|| read_text("/sys/class/dmi/id/product_name"))
            .map(|text| clean(&text))
            .filter(|text| !text.is_empty())
            .or_else(|| model_from_cpuinfo(&cpuinfo));
        let cpu = read_text("/proc/cpuinfo")
            .as_deref()
            .and_then(cpu_model)
            .map(clean)
            .filter(|text| !text.is_empty());
        let cores = core_count(&cpuinfo).unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });
        let memory = read_text("/proc/meminfo")
            .as_deref()
            .and_then(meminfo_bytes);
        let kernel = read_text("/proc/sys/kernel/osrelease").map(|text| clean(&text));
        let system = read_text("/etc/os-release").as_deref().and_then(os_pretty);

        Self {
            model,
            cpu,
            cores,
            memory,
            kernel,
            system,
            arch: std::env::consts::ARCH,
            ip: local_ipv4(),
        }
    }

    /// Lines for the about screen, labels aligned.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(model) = &self.model {
            lines.push(format!("{:<8}{model}", "Device"));
        }
        if let Some(cpu) = &self.cpu {
            lines.push(format!(
                "{:<8}{cpu} \u{00b7} {} {}",
                "CPU",
                self.cores,
                if self.cores == 1 { "core" } else { "cores" }
            ));
        } else {
            lines.push(format!(
                "{:<8}{} {}",
                "CPU",
                self.arch,
                if self.cores == 1 { "core" } else { "cores" }
            ));
        }
        if let Some(memory) = self.memory {
            lines.push(format!("{:<8}{}", "Memory", gigabytes(memory)));
        }
        if let Some(kernel) = &self.kernel {
            lines.push(format!("{:<8}{kernel} \u{00b7} {}", "Kernel", self.arch));
        } else {
            lines.push(format!("{:<8}{}", "Kernel", self.arch));
        }
        if let Some(system) = &self.system {
            lines.push(format!("{:<8}{system}", "System"));
        }
        lines.push(format!(
            "{:<8}{}",
            "Network",
            self.ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "no address".into())
        ));
        lines
    }

    /// The address for the SETTINGS row.
    pub(super) fn ip_label(&self) -> String {
        self.ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "no address".into())
    }
}

fn read_text(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Device-tree strings are NUL-terminated; DMI files end in a newline.
fn clean(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_string()
}

fn model_from_cpuinfo(text: &str) -> Option<String> {
    field(text, "Model").map(|value| value.to_string())
}

fn cpu_model(text: &str) -> Option<&str> {
    field(text, "model name")
        .or_else(|| field(text, "Hardware"))
        .or_else(|| field(text, "Processor"))
        .or_else(|| field(text, "cpu model"))
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == name).then_some(value.trim())
    })
}

fn core_count(text: &str) -> Option<usize> {
    let count = text
        .lines()
        .filter(|line| {
            line.split_once(':')
                .is_some_and(|(key, _)| key.trim() == "processor")
        })
        .count();
    (count > 0).then_some(count)
}

fn meminfo_bytes(text: &str) -> Option<u64> {
    let value = field(text, "MemTotal")?;
    let kib = value.strip_suffix(" kB")?.trim().parse::<u64>().ok()?;
    Some(kib * 1024)
}

fn os_pretty(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "PRETTY_NAME").then(|| value.trim().trim_matches('"').to_string())
    })
}

fn gigabytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GiB", bytes as f64 / GIB)
}

/// The kernel's choice of source address; connecting a UDP socket sends no
/// packet, but it does make the kernel pick the interface it would use.
fn local_ipv4() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 80)).ok()?;
    socket.local_addr().ok().map(|address| address.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI_CPUINFO: &str = "\
processor\t: 0
BogoMIPS\t: 108.00
Features\t: fp asimd evtstrm crc32 cpuid
CPU implementer\t: 0x41
Hardware\t: BCM2711
Revision\t: d03114
Model\t\t: Raspberry Pi 4 Model B Rev 1.4
";

    const X86_CPUINFO: &str = "\
processor\t: 0
model name\t: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
cpu MHz\t\t: 3300.000
processor\t: 1
model name\t: AMD Ryzen 7 7840U w/ Radeon 780M Graphics
";

    #[test]
    fn reads_board_cpu_and_cores() {
        assert_eq!(
            model_from_cpuinfo(PI_CPUINFO).as_deref(),
            Some("Raspberry Pi 4 Model B Rev 1.4")
        );
        assert_eq!(cpu_model(PI_CPUINFO), Some("BCM2711"));
        assert_eq!(core_count(PI_CPUINFO), Some(1));

        assert_eq!(
            cpu_model(X86_CPUINFO),
            Some("AMD Ryzen 7 7840U w/ Radeon 780M Graphics")
        );
        assert_eq!(core_count(X86_CPUINFO), Some(2));
    }

    #[test]
    fn reads_memory_and_os_name() {
        let meminfo = "MemTotal:        3963968 kB\nMemFree:         1234567 kB\n";
        assert_eq!(meminfo_bytes(meminfo), Some(3_963_968 * 1024));
        assert_eq!(gigabytes(4 * 1024 * 1024 * 1024), "4.0 GiB");

        let os = "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian\"\n";
        assert_eq!(
            os_pretty(os).as_deref(),
            Some("Debian GNU/Linux 12 (bookworm)")
        );
    }

    #[test]
    fn the_device_lines_are_labeled_and_network_is_always_there() {
        let machine = Machine {
            model: Some("Raspberry Pi 4 Model B Rev 1.4".into()),
            cpu: Some("Cortex-A72".into()),
            cores: 4,
            memory: Some(4 * 1024 * 1024 * 1024),
            kernel: Some("6.6.51+rpt-rpi-v8".into()),
            system: Some("Debian GNU/Linux 12 (bookworm)".into()),
            arch: "aarch64",
            ip: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))),
        };
        let lines = machine.lines();
        assert!(lines[0].starts_with("Device  Raspberry Pi 4"));
        assert!(lines.iter().any(|line| line.contains("4 cores")));
        assert!(lines.iter().any(|line| line.contains("4.0 GiB")));
        assert!(lines.iter().any(|line| line.contains("aarch64")));
        assert!(lines.iter().any(|line| line.contains("192.168.1.42")));
        assert_eq!(machine.ip_label(), "192.168.1.42");
    }
}
