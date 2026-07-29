use crate::model::{NetworkRecord, PublishedPort};
use crate::util::{run_output, sha256_bytes};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::process::{Command, Stdio};

pub fn default_network(name: &str, published_ports: Vec<PublishedPort>) -> NetworkRecord {
    let digest = sha256_bytes(name.as_bytes());
    let octet = 4 + (u8::from_str_radix(&digest[0..2], 16).unwrap_or(0) % 240);
    let suffix = &digest[..8];
    NetworkRecord {
        tap_name: format!("smp{}", &suffix[..8]),
        guest_mac: format!(
            "06:53:4d:{:02x}:{:02x}:{:02x}",
            octet,
            u8::from_str_radix(&digest[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&digest[4..6], 16).unwrap_or(0)
        ),
        guest_address: format!("172.31.{octet}.2"),
        gateway_address: format!("172.31.{octet}.1"),
        prefix_length: 30,
        dns_servers: vec!["1.1.1.1".to_owned(), "1.0.0.1".to_owned()],
        published_ports,
        managed: true,
    }
}

pub fn check_collision(network: &NetworkRecord) -> Result<()> {
    let output = run_output("ip", &[OsString::from("link"), OsString::from("show"), OsString::from(&network.tap_name)])?;
    if output.status.success() {
        bail!("network collision: TAP {} already exists", network.tap_name);
    }
    let output = run_output(
        "ip",
        &[
            OsString::from("-o"),
            OsString::from("addr"),
            OsString::from("show"),
            OsString::from("to"),
            OsString::from(format!("{}/{}", network.gateway_address, network.prefix_length)),
        ],
    )?;
    if output.status.success() && !output.stdout.is_empty() {
        bail!("network collision: gateway {} is already assigned", network.gateway_address);
    }
    Ok(())
}

pub fn create(name: &str, network: &NetworkRecord) -> Result<()> {
    if !network.managed {
        return Ok(());
    }
    check_collision(network)?;
    run_checked("ip", &["tuntap", "add", "dev", &network.tap_name, "mode", "tap"])?;
    let result = (|| {
        run_checked(
            "ip",
            &[
                "addr",
                "add",
                &format!("{}/{}", network.gateway_address, network.prefix_length),
                "dev",
                &network.tap_name,
            ],
        )?;
        run_checked("ip", &["link", "set", "dev", &network.tap_name, "up"])?;
        install_nftables(name, network)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = cleanup(name, network);
    }
    result
}

pub fn cleanup(name: &str, network: &NetworkRecord) -> Result<()> {
    if !network.managed {
        return Ok(());
    }
    let table = table_name(name);
    let _ = run_checked("nft", &["delete", "table", "ip", &table]);
    let _ = run_checked("ip", &["link", "delete", "dev", &network.tap_name]);
    Ok(())
}

pub fn exists(network: &NetworkRecord) -> bool {
    Command::new("ip")
        .args(["link", "show", "dev", &network.tap_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn install_nftables(name: &str, network: &NetworkRecord) -> Result<()> {
    let table = table_name(name);
    let outbound = default_route_interface()?;
    let mut script = format!(
        "table ip {table} {{\n\
         chain prerouting {{ type nat hook prerouting priority dstnat; policy accept; }}\n\
         chain forward {{ type filter hook forward priority filter; policy accept; \
         iifname \"{}\" accept; oifname \"{}\" ct state established,related accept; }}\n\
         chain postrouting {{ type nat hook postrouting priority srcnat; policy accept; \
         ip saddr {}/{} oifname \"{}\" masquerade; }}\n",
        network.tap_name,
        network.tap_name,
        subnet(network),
        network.prefix_length,
        outbound
    );
    for port in &network.published_ports {
        let protocol = match port.protocol.as_str() {
            "tcp" => "tcp",
            "udp" => "udp",
            other => bail!("unsupported port protocol {other}"),
        };
        script.push_str(&format!(
            "add rule ip {table} prerouting {protocol} dport {} dnat to {}:{}\n",
            port.host_port, network.guest_address, port.guest_port
        ));
    }
    script.push_str("}\n");

    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start nft")?;
    use std::io::Write;
    child.stdin.as_mut().expect("piped stdin").write_all(script.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("nftables setup failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn default_route_interface() -> Result<String> {
    let output = Command::new("ip")
        .args(["-o", "route", "show", "default"])
        .output()
        .context("inspect default route")?;
    if !output.status.success() {
        bail!("cannot inspect default route: {}", String::from_utf8_lossy(&output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let fields: Vec<&str> = text.split_whitespace().collect();
    fields
        .windows(2)
        .find(|window| window[0] == "dev")
        .map(|window| window[1].to_owned())
        .ok_or_else(|| anyhow::anyhow!("default route has no device"))
}

fn table_name(name: &str) -> String {
    let digest = sha256_bytes(name.as_bytes());
    format!("smp_{}", &digest[..12])
}

fn subnet(network: &NetworkRecord) -> String {
    network.gateway_address.trim_end_matches(".1").to_owned() + ".0"
}

fn run_checked(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        bail!(
            "{} {} failed: {}",
            program,
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_network_is_deterministic() {
        assert_eq!(default_network("default", vec![]).tap_name, default_network("default", vec![]).tap_name);
        assert_ne!(default_network("default", vec![]).guest_address, default_network("other", vec![]).guest_address);
    }

    #[test]
    fn tap_name_fits_linux_limit() {
        assert!(default_network("default", vec![]).tap_name.len() <= 15);
    }
}
