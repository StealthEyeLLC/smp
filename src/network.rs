use crate::error::{Result, SmpError};
use crate::model::{NetworkDefinition, PortProtocol, PortPublication};
use crate::util::command_output;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::net::Ipv4Addr;
use std::process::Output;

const TABLE_FAMILY: &str = "inet";
const TABLE_NAME: &str = "smp";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPlan {
    pub definition: NetworkDefinition,
    pub apply_commands: Vec<Vec<String>>,
    pub cleanup_commands: Vec<Vec<String>>,
}

pub fn deterministic_definition(
    machine: &str,
    existing: &[NetworkDefinition],
    published_ports: Vec<PortPublication>,
) -> Result<NetworkDefinition> {
    validate_ports(&published_ports, existing)?;
    let digest = Sha256::digest(machine.as_bytes());
    let tap = format!("smp{}", &hex::encode(digest)[..10]);
    if tap.len() > 15 {
        return Err(SmpError::State(format!("TAP name too long: {tap}")));
    }
    if existing.iter().any(|network| network.tap == tap) {
        return Err(SmpError::Conflict(format!("TAP collision for {tap}")));
    }
    let used = existing
        .iter()
        .filter_map(|network| network.subnet.split('.').nth(2))
        .filter_map(|octet| octet.parse::<u8>().ok())
        .collect::<HashSet<_>>();
    let start = digest[10].max(1);
    let octet = (0_u16..254)
        .map(|offset| ((u16::from(start) + offset - 1) % 254 + 1) as u8)
        .find(|candidate| !used.contains(candidate))
        .ok_or_else(|| SmpError::Conflict("no deterministic SMP subnet remains".to_owned()))?;
    let mac = format!(
        "06:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        digest[11], digest[12], digest[13], digest[14], digest[15]
    );
    Ok(NetworkDefinition {
        tap,
        subnet: format!("172.31.{octet}.0"),
        prefix_length: 30,
        guest_address: format!("172.31.{octet}.2"),
        gateway: format!("172.31.{octet}.1"),
        dns: vec!["1.1.1.1".to_owned(), "9.9.9.9".to_owned()],
        guest_mac: mac,
        published_ports,
    })
}

pub fn validate_definition(definition: &NetworkDefinition) -> Result<()> {
    if definition.tap.is_empty()
        || definition.tap.len() > 15
        || !definition
            .tap
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(SmpError::Invalid(format!(
            "invalid TAP name {}",
            definition.tap
        )));
    }
    if definition.prefix_length > 32 {
        return Err(SmpError::Invalid("invalid IPv4 prefix".to_owned()));
    }
    for address in [
        &definition.subnet,
        &definition.guest_address,
        &definition.gateway,
    ] {
        address
            .parse::<Ipv4Addr>()
            .map_err(|_| SmpError::Invalid(format!("invalid IPv4 address {address}")))?;
    }
    if !is_mac(&definition.guest_mac) {
        return Err(SmpError::Invalid(format!(
            "invalid guest MAC {}",
            definition.guest_mac
        )));
    }
    validate_ports(&definition.published_ports, &[])
}

pub fn plan(machine: &str, definition: &NetworkDefinition) -> Result<NetworkPlan> {
    validate_definition(definition)?;
    let suffix = chain_suffix(machine);
    let forward = format!("f_{suffix}");
    let postrouting = format!("p_{suffix}");
    let prerouting = format!("n_{suffix}");
    let output = format!("o_{suffix}");
    let comment = format!("smp:{machine}");
    let mut apply = vec![
        vec![
            "ip".into(),
            "tuntap".into(),
            "add".into(),
            "dev".into(),
            definition.tap.clone(),
            "mode".into(),
            "tap".into(),
            "user".into(),
            "0".into(),
        ],
        vec![
            "ip".into(),
            "link".into(),
            "set".into(),
            "dev".into(),
            definition.tap.clone(),
            "alias".into(),
            format!("{comment}:forward-out"),
        ],
        vec![
            "ip".into(),
            "address".into(),
            "replace".into(),
            format!("{}/{}", definition.gateway, definition.prefix_length),
            "dev".into(),
            definition.tap.clone(),
        ],
        vec![
            "ip".into(),
            "link".into(),
            "set".into(),
            "dev".into(),
            definition.tap.clone(),
            "up".into(),
        ],
        nft_chain(&forward, "filter", "forward", "-10"),
        nft_chain(&postrouting, "nat", "postrouting", "srcnat"),
        nft_chain(&prerouting, "nat", "prerouting", "dstnat"),
        nft_chain(&output, "nat", "output", "-100"),
        vec![
            "nft".into(),
            "add".into(),
            "rule".into(),
            TABLE_FAMILY.into(),
            TABLE_NAME.into(),
            forward.clone(),
            "iifname".into(),
            definition.tap.clone(),
            "accept".into(),
            "comment".into(),
            format!("{comment}:forward-return"),
        ],
        vec![
            "nft".into(),
            "add".into(),
            "rule".into(),
            TABLE_FAMILY.into(),
            TABLE_NAME.into(),
            forward.clone(),
            "oifname".into(),
            definition.tap.clone(),
            "ct".into(),
            "state".into(),
            "established,related".into(),
            "accept".into(),
            "comment".into(),
            format!("{comment}:masquerade"),
        ],
        vec![
            "nft".into(),
            "add".into(),
            "rule".into(),
            TABLE_FAMILY.into(),
            TABLE_NAME.into(),
            postrouting.clone(),
            "ip".into(),
            "saddr".into(),
            format!("{}/{}", definition.subnet, definition.prefix_length),
            "oifname".into(),
            "!=".into(),
            definition.tap.clone(),
            "masquerade".into(),
            "comment".into(),
            format!("{comment}:hairpin"),
        ],
        vec![
            "nft".into(),
            "add".into(),
            "rule".into(),
            TABLE_FAMILY.into(),
            TABLE_NAME.into(),
            postrouting.clone(),
            "ip".into(),
            "saddr".into(),
            "127.0.0.0/8".into(),
            "ip".into(),
            "daddr".into(),
            definition.guest_address.clone(),
            "snat".into(),
            "to".into(),
            definition.gateway.clone(),
            "comment".into(),
            comment.clone(),
        ],
    ];
    for port in &definition.published_ports {
        let protocol = protocol_name(&port.protocol);
        for (chain, local_only) in [(&prerouting, false), (&output, true)] {
            let mut command = vec![
                "nft".into(),
                "add".into(),
                "rule".into(),
                TABLE_FAMILY.into(),
                TABLE_NAME.into(),
                chain.clone(),
            ];
            if local_only {
                command.extend(["ip".into(), "daddr".into(), port.bind_address.clone()]);
            }
            command.extend([
                protocol.into(),
                "dport".into(),
                port.host_port.to_string(),
                "dnat".into(),
                "to".into(),
                format!("{}:{}", definition.guest_address, port.guest_port),
                "comment".into(),
                format!(
                    "{comment}:{}:{}:{}:{}",
                    protocol,
                    port.bind_address,
                    port.host_port,
                    if local_only { "output" } else { "prerouting" }
                ),
            ]);
            apply.push(command);
        }
    }
    for command in &mut apply {
        if let Some(index) = command.iter().position(|value| value == "comment")
            && let Some(value) = command.get_mut(index + 1)
        {
            *value = nft_comment(value);
        }
    }
    let cleanup = vec![
        nft_delete_chain(&output),
        nft_delete_chain(&prerouting),
        nft_delete_chain(&postrouting),
        nft_delete_chain(&forward),
        vec![
            "ip".into(),
            "link".into(),
            "delete".into(),
            "dev".into(),
            definition.tap.clone(),
        ],
    ];
    Ok(NetworkPlan {
        definition: definition.clone(),
        apply_commands: apply,
        cleanup_commands: cleanup,
    })
}

pub fn apply(machine: &str, definition: &NetworkDefinition) -> Result<()> {
    let forwarding = fs::read_to_string("/proc/sys/net/ipv4/ip_forward");
    if !matches!(forwarding, Ok(value) if value.trim() == "1") {
        return Err(SmpError::State(
            "IPv4 forwarding is disabled; run smp doctor --fix".to_owned(),
        ));
    }
    let plan = plan(machine, definition)?;
    ensure_table()?;
    let alias = format!("smp:{machine}");
    if let Some(existing_alias) = link_alias(&definition.tap)? {
        if existing_alias != alias {
            return Err(SmpError::Ambiguous(format!(
                "TAP {} exists with alias {}",
                definition.tap, existing_alias
            )));
        }
    } else {
        run(&plan.apply_commands[0])?;
        run(&plan.apply_commands[1])?;
    }
    run(&plan.apply_commands[2])?;
    run(&plan.apply_commands[3])?;
    for command in &plan.apply_commands[4..] {
        let chain = command.get(5).cloned().unwrap_or_default();
        if command.get(1).is_some_and(|value| value == "add")
            && command.get(2).is_some_and(|value| value == "chain")
            && nft_chain_exists(&chain)?
        {
            continue;
        }
        if command.get(2).is_some_and(|value| value == "rule")
            && nft_chain_exists(&chain)?
            && let Some(comment) = command.last()
            && nft_chain_has_comment(&chain, comment)?
        {
            continue;
        }
        run(command)?;
    }
    Ok(())
}

pub fn cleanup(machine: &str, definition: &NetworkDefinition) -> Result<()> {
    let plan = plan(machine, definition)?;
    for command in &plan.cleanup_commands[..4] {
        if let Some(chain) = command.get(5)
            && nft_chain_exists(chain)?
        {
            run(command)?;
        }
    }
    if let Some(alias) = link_alias(&definition.tap)? {
        let expected = format!("smp:{machine}");
        if alias != expected {
            return Err(SmpError::Ambiguous(format!(
                "refusing to delete TAP {} with alias {}",
                definition.tap, alias
            )));
        }
        run(&plan.cleanup_commands[4])?;
    }
    Ok(())
}

fn validate_ports(ports: &[PortPublication], existing: &[NetworkDefinition]) -> Result<()> {
    let mut seen = HashSet::new();
    for port in ports {
        if port.host_port == 0 || port.guest_port == 0 {
            return Err(SmpError::Invalid("port zero is unsupported".to_owned()));
        }
        port.bind_address.parse::<Ipv4Addr>().map_err(|_| {
            SmpError::Invalid(format!("invalid bind address {}", port.bind_address))
        })?;
        let key = (
            port.protocol.clone(),
            port.bind_address.clone(),
            port.host_port,
        );
        if !seen.insert(key.clone())
            || existing
                .iter()
                .flat_map(|network| &network.published_ports)
                .any(|item| {
                    (
                        item.protocol.clone(),
                        item.bind_address.clone(),
                        item.host_port,
                    ) == key
                })
        {
            return Err(SmpError::Conflict(format!(
                "host port collision on {}:{}",
                port.bind_address, port.host_port
            )));
        }
    }
    Ok(())
}

fn chain_suffix(machine: &str) -> String {
    let digest = Sha256::digest(machine.as_bytes());
    hex::encode(digest)[..10].to_owned()
}

fn nft_comment(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn nft_chain(name: &str, kind: &str, hook: &str, priority: &str) -> Vec<String> {
    vec![
        "nft".into(),
        "add".into(),
        "chain".into(),
        TABLE_FAMILY.into(),
        TABLE_NAME.into(),
        name.into(),
        "{".into(),
        "type".into(),
        kind.into(),
        "hook".into(),
        hook.into(),
        "priority".into(),
        format!("{priority};"),
        "policy".into(),
        "accept;".into(),
        "}".into(),
    ]
}

fn nft_delete_chain(name: &str) -> Vec<String> {
    vec![
        "nft".into(),
        "delete".into(),
        "chain".into(),
        TABLE_FAMILY.into(),
        TABLE_NAME.into(),
        name.into(),
    ]
}

fn ensure_table() -> Result<()> {
    let check = strings(&["nft", "list", "table", TABLE_FAMILY, TABLE_NAME]);
    if !run_output(&check)?.status.success() {
        run(&strings(&["nft", "add", "table", TABLE_FAMILY, TABLE_NAME]))?;
    }
    Ok(())
}

fn nft_chain_exists(chain: &str) -> Result<bool> {
    Ok(run_output(&strings(&[
        "nft",
        "list",
        "chain",
        TABLE_FAMILY,
        TABLE_NAME,
        chain,
    ]))?
    .status
    .success())
}

fn nft_chain_has_comment(chain: &str, comment: &str) -> Result<bool> {
    let output = run_output(&strings(&[
        "nft",
        "list",
        "chain",
        TABLE_FAMILY,
        TABLE_NAME,
        chain,
    ]))?;
    Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains(&format!("comment {comment}")))
}

fn link_alias(tap: &str) -> Result<Option<String>> {
    let output = run_output(&[
        "ip".into(),
        "-j".into(),
        "-d".into(),
        "link".into(),
        "show".into(),
        "dev".into(),
        tap.into(),
    ])?;
    if !output.status.success() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| SmpError::json("ip -j link", error))?;
    Ok(value
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("ifalias"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned))
}

fn run(command: &[String]) -> Result<()> {
    let output = run_output(command)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SmpError::External {
            program: command.join(" "),
            code: output.status.code().unwrap_or(128),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn run_output(command: &[String]) -> Result<Output> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| SmpError::Invalid("empty network command".to_owned()))?;
    command_output(program, arguments)
}

fn protocol_name(protocol: &PortProtocol) -> &'static str {
    match protocol {
        PortProtocol::Tcp => "tcp",
        PortProtocol::Udp => "udp",
    }
}

fn is_mac(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && u8::from_str_radix(part, 16).is_ok())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_network_identity_and_limits() -> Result<()> {
        let first = deterministic_definition("default", &[], vec![])?;
        let second = deterministic_definition("default", &[], vec![])?;
        assert_eq!(first, second);
        assert!(first.tap.len() <= 15);
        assert!(first.guest_mac.starts_with("06:"));
        Ok(())
    }

    #[test]
    fn multiple_machines_receive_isolated_subnets() -> Result<()> {
        let first = deterministic_definition("one", &[], vec![])?;
        let second = deterministic_definition("two", std::slice::from_ref(&first), vec![])?;
        assert_ne!(first.tap, second.tap);
        assert_ne!(first.subnet, second.subnet);
        Ok(())
    }

    #[test]
    fn host_port_collisions_are_rejected() -> Result<()> {
        let port = PortPublication {
            protocol: PortProtocol::Tcp,
            bind_address: "127.0.0.1".to_owned(),
            host_port: 8080,
            guest_port: 80,
        };
        let first = deterministic_definition("one", &[], vec![port.clone()])?;
        assert!(deterministic_definition("two", &[first], vec![port]).is_err());
        Ok(())
    }

    #[test]
    fn network_plan_contains_nat_hairpin_and_owned_cleanup() -> Result<()> {
        let definition = deterministic_definition(
            "default",
            &[],
            vec![PortPublication {
                protocol: PortProtocol::Tcp,
                bind_address: "127.0.0.1".to_owned(),
                host_port: 8080,
                guest_port: 80,
            }],
        )?;
        let plan = plan("default", &definition)?;
        let rendered = plan
            .apply_commands
            .iter()
            .map(|command| command.join(" "))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("masquerade"));
        assert!(rendered.contains("127.0.0.0/8"));
        assert!(rendered.contains("dnat"));
        assert!(rendered.contains("smp:default"));
        let comments = plan
            .apply_commands
            .iter()
            .filter_map(|command| {
                command
                    .iter()
                    .position(|value| value == "comment")
                    .and_then(|index| command.get(index + 1))
            })
            .collect::<Vec<_>>();
        assert!(!comments.is_empty());
        assert!(
            comments
                .iter()
                .all(|value| value.starts_with('"') && value.ends_with('"'))
        );
        assert_eq!(
            plan.cleanup_commands.last().and_then(|value| value.last()),
            Some(&definition.tap)
        );
        Ok(())
    }
}
