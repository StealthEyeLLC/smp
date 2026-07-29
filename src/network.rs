use crate::model::{NetworkRecord, PublishedPort};
use crate::util::{run_output, sha256_bytes};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::{Command, Output, Stdio};

const IPTABLES_WAIT_SECONDS: &str = "5";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainPlan {
    table: &'static str,
    builtin: &'static str,
    owned: String,
    jump_comment: String,
    rules: Vec<Vec<String>>,
}

pub fn default_network(name: &str, published_ports: Vec<PublishedPort>) -> NetworkRecord {
    let digest = sha256_bytes(name.as_bytes());
    let octet = 4 + (u8::from_str_radix(&digest[0..2], 16).unwrap_or(0) % 240);
    NetworkRecord {
        tap_name: format!("smp{}", &digest[..8]),
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

pub fn create(name: &str, network: &NetworkRecord) -> Result<()> {
    if !network.managed {
        return Ok(());
    }
    cleanup_owned_chains(name)?;
    remove_legacy_nft_table(name)?;
    remove_legacy_direct_rules(network)?;
    check_collision(network)?;

    run_checked(
        "ip",
        &["tuntap", "add", "dev", &network.tap_name, "mode", "tap"],
    )?;
    let result = (|| {
        let address = format!("{}/{}", network.gateway_address, network.prefix_length);
        run_checked("ip", &["addr", "add", &address, "dev", &network.tap_name])?;
        run_checked("ip", &["link", "set", "dev", &network.tap_name, "up"])?;
        configure_host_network(name, network)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup_error) = cleanup(name, network) {
            bail!("host network setup failed: {error:#}; cleanup also failed: {cleanup_error:#}");
        }
        return Err(error);
    }
    Ok(())
}

pub fn cleanup(name: &str, network: &NetworkRecord) -> Result<()> {
    if !network.managed {
        return Ok(());
    }
    cleanup_host_rules(name, network)?;
    if exists(network) {
        run_checked("ip", &["link", "delete", "dev", &network.tap_name])
    } else {
        Ok(())
    }
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

fn check_collision(network: &NetworkRecord) -> Result<()> {
    validate_published_ports(network)?;
    let output = run_output(
        "ip",
        &[
            OsString::from("link"),
            OsString::from("show"),
            OsString::from(&network.tap_name),
        ],
    )?;
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
            OsString::from(format!(
                "{}/{}",
                network.gateway_address, network.prefix_length
            )),
        ],
    )?;
    if output.status.success() && !output.stdout.is_empty() {
        bail!(
            "network collision: gateway {} is already assigned",
            network.gateway_address
        );
    }
    check_published_port_collisions(network)
}

fn configure_host_network(name: &str, network: &NetworkRecord) -> Result<()> {
    ensure_sysctls(network)?;
    let outbound = default_route_interface()?;
    cleanup_owned_chains(name)?;
    for plan in chain_plans(name, network, &outbound)? {
        install_chain(&plan)?;
    }
    Ok(())
}

fn ensure_sysctls(network: &NetworkRecord) -> Result<()> {
    for setting in [
        "net.ipv4.ip_forward=1".to_owned(),
        "net.ipv4.conf.default.rp_filter=0".to_owned(),
        format!("net.ipv4.conf.{}.rp_filter=0", network.tap_name),
    ] {
        run_checked("sysctl", &["-q", "-w", &setting])?;
    }
    if !network.published_ports.is_empty() {
        for setting in [
            "net.ipv4.conf.all.route_localnet=1".to_owned(),
            "net.ipv4.conf.default.route_localnet=1".to_owned(),
            format!("net.ipv4.conf.{}.route_localnet=1", network.tap_name),
        ] {
            run_checked("sysctl", &["-q", "-w", &setting])?;
        }
    }
    Ok(())
}

fn chain_plans(name: &str, network: &NetworkRecord, outbound: &str) -> Result<Vec<ChainPlan>> {
    validate_published_ports(network)?;
    let suffix = chain_suffix(name);
    let tag = format!("smp:{suffix}");
    let subnet = subnet_cidr(network);

    let input = vec![commented_rule(
        vec![
            "-i",
            &network.tap_name,
            "-s",
            &network.guest_address,
            "-m",
            "conntrack",
            "--ctstate",
            "ESTABLISHED,RELATED",
        ],
        &format!("{tag}:host-input"),
        vec!["-j", "ACCEPT"],
    )];
    let output = vec![commented_rule(
        vec!["-o", &network.tap_name, "-d", &network.guest_address],
        &format!("{tag}:host-output"),
        vec!["-j", "ACCEPT"],
    )];
    let mut forward = vec![
        commented_rule(
            vec!["-i", &network.tap_name, "-s", &subnet],
            &format!("{tag}:guest-forward"),
            vec!["-j", "ACCEPT"],
        ),
        commented_rule(
            vec![
                "-o",
                &network.tap_name,
                "-d",
                &subnet,
                "-m",
                "conntrack",
                "--ctstate",
                "ESTABLISHED,RELATED",
            ],
            &format!("{tag}:guest-return"),
            vec!["-j", "ACCEPT"],
        ),
    ];
    let mut prerouting = Vec::new();
    let mut nat_output = Vec::new();
    let mut postrouting = vec![commented_rule(
        vec!["-s", &subnet, "-o", outbound],
        &format!("{tag}:masquerade"),
        vec!["-j", "MASQUERADE"],
    )];

    for port in &network.published_ports {
        let protocol = checked_protocol(&port.protocol)?;
        let host_port = port.host_port.to_string();
        let guest_port = port.guest_port.to_string();
        let destination = format!("{}:{}", network.guest_address, port.guest_port);
        let port_tag = format!("{tag}:{protocol}:{}:{}", port.host_port, port.guest_port);
        forward.push(commented_rule(
            vec![
                "-o",
                &network.tap_name,
                "-p",
                protocol,
                "-d",
                &network.guest_address,
                "--dport",
                &guest_port,
                "-m",
                "conntrack",
                "--ctstate",
                "NEW,ESTABLISHED,RELATED",
            ],
            &format!("{port_tag}:forward"),
            vec!["-j", "ACCEPT"],
        ));
        prerouting.push(commented_rule(
            vec!["-p", protocol, "--dport", &host_port],
            &format!("{port_tag}:prerouting"),
            vec!["-j", "DNAT", "--to-destination", &destination],
        ));
        nat_output.push(commented_rule(
            vec![
                "-p",
                protocol,
                "-m",
                "addrtype",
                "--dst-type",
                "LOCAL",
                "--dport",
                &host_port,
            ],
            &format!("{port_tag}:output"),
            vec!["-j", "DNAT", "--to-destination", &destination],
        ));
        postrouting.push(commented_rule(
            vec![
                "-p",
                protocol,
                "-s",
                "127.0.0.0/8",
                "-d",
                &network.guest_address,
                "--dport",
                &guest_port,
            ],
            &format!("{port_tag}:hairpin"),
            vec!["-j", "SNAT", "--to-source", &network.gateway_address],
        ));
    }

    Ok(vec![
        chain_plan("filter", "INPUT", &suffix, "I", input),
        chain_plan("filter", "OUTPUT", &suffix, "O", output),
        chain_plan("filter", "FORWARD", &suffix, "F", forward),
        chain_plan("nat", "PREROUTING", &suffix, "PR", prerouting),
        chain_plan("nat", "OUTPUT", &suffix, "NO", nat_output),
        chain_plan("nat", "POSTROUTING", &suffix, "PO", postrouting),
    ])
}

fn chain_plan(
    table: &'static str,
    builtin: &'static str,
    suffix: &str,
    code: &str,
    rules: Vec<Vec<String>>,
) -> ChainPlan {
    ChainPlan {
        table,
        builtin,
        owned: format!("SMP_{code}_{suffix}"),
        jump_comment: format!("smp:{suffix}:jump:{}", builtin.to_ascii_lowercase()),
        rules,
    }
}

fn commented_rule(matches: Vec<&str>, comment: &str, target: Vec<&str>) -> Vec<String> {
    matches
        .into_iter()
        .chain(["-m", "comment", "--comment", comment])
        .chain(target)
        .map(str::to_owned)
        .collect()
}

fn install_chain(plan: &ChainPlan) -> Result<()> {
    if !iptables_success(plan.table, &["-L", &plan.owned, "-n"])? {
        iptables_checked(plan.table, &["-N", &plan.owned])?;
    }
    for rule in &plan.rules {
        let mut check = vec!["-C".to_owned(), plan.owned.clone()];
        check.extend(rule.clone());
        if !iptables_success_owned(plan.table, &check)? {
            let mut append = vec!["-A".to_owned(), plan.owned.clone()];
            append.extend(rule.clone());
            iptables_checked_owned(plan.table, &append)?;
        }
    }
    let jump = jump_rule(plan);
    let mut check = vec!["-C".to_owned(), plan.builtin.to_owned()];
    check.extend(jump.clone());
    if !iptables_success_owned(plan.table, &check)? {
        let mut insert = vec!["-I".to_owned(), plan.builtin.to_owned(), "1".to_owned()];
        insert.extend(jump);
        iptables_checked_owned(plan.table, &insert)?;
    }
    Ok(())
}

fn cleanup_host_rules(name: &str, network: &NetworkRecord) -> Result<()> {
    cleanup_owned_chains(name)?;
    remove_legacy_nft_table(name)?;
    remove_legacy_direct_rules(network)
}

fn cleanup_owned_chains(name: &str) -> Result<()> {
    for plan in chain_bindings(name) {
        let jump = jump_rule(&plan);
        let mut check = vec!["-C".to_owned(), plan.builtin.to_owned()];
        check.extend(jump.clone());
        while iptables_success_owned(plan.table, &check)? {
            let mut delete = vec!["-D".to_owned(), plan.builtin.to_owned()];
            delete.extend(jump.clone());
            iptables_checked_owned(plan.table, &delete)?;
        }
        if iptables_success(plan.table, &["-L", &plan.owned, "-n"])? {
            iptables_checked(plan.table, &["-F", &plan.owned])?;
            iptables_checked(plan.table, &["-X", &plan.owned])?;
        }
    }
    Ok(())
}

fn chain_bindings(name: &str) -> Vec<ChainPlan> {
    let suffix = chain_suffix(name);
    vec![
        chain_plan("filter", "INPUT", &suffix, "I", Vec::new()),
        chain_plan("filter", "OUTPUT", &suffix, "O", Vec::new()),
        chain_plan("filter", "FORWARD", &suffix, "F", Vec::new()),
        chain_plan("nat", "PREROUTING", &suffix, "PR", Vec::new()),
        chain_plan("nat", "OUTPUT", &suffix, "NO", Vec::new()),
        chain_plan("nat", "POSTROUTING", &suffix, "PO", Vec::new()),
    ]
}

fn jump_rule(plan: &ChainPlan) -> Vec<String> {
    [
        "-m".to_owned(),
        "comment".to_owned(),
        "--comment".to_owned(),
        plan.jump_comment.clone(),
        "-j".to_owned(),
        plan.owned.clone(),
    ]
    .to_vec()
}

fn remove_legacy_nft_table(name: &str) -> Result<()> {
    let table = legacy_table_name(name);
    let output = Command::new("nft")
        .args(["list", "tables"])
        .output()
        .context("inspect nftables tables")?;
    if !output.status.success() {
        bail!(
            "nft list tables failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let declaration = format!("table ip {table}");
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == declaration)
    {
        run_checked("nft", &["delete", "table", "ip", &table])?;
    }
    Ok(())
}

fn remove_legacy_direct_rules(network: &NetworkRecord) -> Result<()> {
    let subnet = subnet_cidr(network);
    delete_rule_all(
        "filter",
        "OUTPUT",
        &["-o", &network.tap_name, "-j", "ACCEPT"],
    )?;
    delete_rule_all(
        "filter",
        "INPUT",
        &[
            "-i",
            &network.tap_name,
            "-m",
            "conntrack",
            "--ctstate",
            "ESTABLISHED,RELATED",
            "-j",
            "ACCEPT",
        ],
    )?;
    delete_rule_all(
        "filter",
        "FORWARD",
        &["-i", &network.tap_name, "-j", "ACCEPT"],
    )?;
    delete_rule_all(
        "filter",
        "FORWARD",
        &[
            "-o",
            &network.tap_name,
            "-m",
            "conntrack",
            "--ctstate",
            "ESTABLISHED,RELATED",
            "-j",
            "ACCEPT",
        ],
    )?;
    if let Ok(outbound) = default_route_interface() {
        delete_rule_all(
            "nat",
            "POSTROUTING",
            &["-s", &subnet, "-o", &outbound, "-j", "MASQUERADE"],
        )?;
    }
    for port in &network.published_ports {
        let protocol = checked_protocol(&port.protocol)?;
        let host_port = port.host_port.to_string();
        let guest_port = port.guest_port.to_string();
        let destination = format!("{}:{}", network.guest_address, port.guest_port);
        delete_rule_all(
            "filter",
            "FORWARD",
            &[
                "-o",
                &network.tap_name,
                "-p",
                protocol,
                "-d",
                &network.guest_address,
                "--dport",
                &guest_port,
                "-m",
                "conntrack",
                "--ctstate",
                "NEW,ESTABLISHED,RELATED",
                "-j",
                "ACCEPT",
            ],
        )?;
        delete_rule_all(
            "nat",
            "OUTPUT",
            &[
                "-p",
                protocol,
                "-m",
                "addrtype",
                "--dst-type",
                "LOCAL",
                "--dport",
                &host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &destination,
            ],
        )?;
        delete_rule_all(
            "nat",
            "POSTROUTING",
            &[
                "-p",
                protocol,
                "-s",
                "127.0.0.0/8",
                "-d",
                &network.guest_address,
                "--dport",
                &guest_port,
                "-j",
                "SNAT",
                "--to-source",
                &network.gateway_address,
            ],
        )?;
    }
    Ok(())
}

fn delete_rule_all(table: &str, chain: &str, rule: &[&str]) -> Result<()> {
    let mut check = vec!["-C".to_owned(), chain.to_owned()];
    check.extend(rule.iter().map(|value| (*value).to_owned()));
    while iptables_success_owned(table, &check)? {
        let mut delete = vec!["-D".to_owned(), chain.to_owned()];
        delete.extend(rule.iter().map(|value| (*value).to_owned()));
        iptables_checked_owned(table, &delete)?;
    }
    Ok(())
}

fn check_published_port_collisions(network: &NetworkRecord) -> Result<()> {
    if network.published_ports.is_empty() {
        return Ok(());
    }
    let output = Command::new("iptables-save")
        .args(["-t", "nat"])
        .output()
        .context("inspect host NAT rules")?;
    if !output.status.success() {
        bail!(
            "iptables-save -t nat failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for port in &network.published_ports {
        let protocol = checked_protocol(&port.protocol)?;
        let host_port = port.host_port.to_string();
        for line in text.lines() {
            let tokens = line.split_whitespace().collect::<Vec<_>>();
            if has_pair(&tokens, "-j", "DNAT")
                && has_pair(&tokens, "-p", protocol)
                && has_pair(&tokens, "--dport", &host_port)
            {
                bail!(
                    "published {protocol} host port {} collides with an existing DNAT rule",
                    port.host_port
                );
            }
        }
    }
    Ok(())
}

fn validate_published_ports(network: &NetworkRecord) -> Result<()> {
    let mut seen = BTreeSet::new();
    for port in &network.published_ports {
        let protocol = checked_protocol(&port.protocol)?;
        if port.host_port == 0 || port.guest_port == 0 {
            bail!("published ports must be between 1 and 65535");
        }
        if !seen.insert((protocol.to_owned(), port.host_port)) {
            bail!(
                "duplicate published {protocol} host port {} in machine definition",
                port.host_port
            );
        }
    }
    Ok(())
}

fn has_pair(tokens: &[&str], key: &str, value: &str) -> bool {
    tokens
        .windows(2)
        .any(|window| window[0] == key && window[1] == value)
}

fn checked_protocol(protocol: &str) -> Result<&str> {
    match protocol {
        "tcp" | "udp" => Ok(protocol),
        other => bail!("unsupported port protocol {other}"),
    }
}

fn chain_suffix(name: &str) -> String {
    sha256_bytes(name.as_bytes())[..10].to_owned()
}

fn subnet_cidr(network: &NetworkRecord) -> String {
    format!("{}/{}", subnet(network), network.prefix_length)
}

fn subnet(network: &NetworkRecord) -> String {
    let mut octets = network.gateway_address.split('.');
    let a = octets.next().unwrap_or("0");
    let b = octets.next().unwrap_or("0");
    let c = octets.next().unwrap_or("0");
    format!("{a}.{b}.{c}.0")
}

fn legacy_table_name(name: &str) -> String {
    let digest = sha256_bytes(name.as_bytes());
    format!("smp_{}", &digest[..12])
}

fn default_route_interface() -> Result<String> {
    let output = Command::new("ip")
        .args(["-o", "route", "show", "default"])
        .output()
        .context("inspect default route")?;
    if !output.status.success() {
        bail!(
            "cannot inspect default route: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|window| window[0] == "dev")
        .map(|window| window[1].to_owned())
        .ok_or_else(|| anyhow::anyhow!("default route has no device"))
}

fn iptables_success(table: &str, args: &[&str]) -> Result<bool> {
    let owned = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    iptables_success_owned(table, &owned)
}

fn iptables_success_owned(table: &str, args: &[String]) -> Result<bool> {
    Ok(run_iptables(table, args)?.status.success())
}

fn iptables_checked(table: &str, args: &[&str]) -> Result<()> {
    let owned = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    iptables_checked_owned(table, &owned)
}

fn iptables_checked_owned(table: &str, args: &[String]) -> Result<()> {
    let output = run_iptables(table, args)?;
    if !output.status.success() {
        bail!(
            "iptables {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn run_iptables(table: &str, args: &[String]) -> Result<Output> {
    let mut command = Command::new("iptables");
    command.args(["-w", IPTABLES_WAIT_SECONDS]);
    if table != "filter" {
        command.args(["-t", table]);
    }
    command.args(args);
    command.output().context("run iptables")
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

    fn published_network(name: &str) -> NetworkRecord {
        default_network(
            name,
            vec![PublishedPort {
                protocol: "tcp".to_owned(),
                host_port: 18080,
                guest_port: 8080,
            }],
        )
    }

    fn plans(name: &str, network: &NetworkRecord) -> Vec<ChainPlan> {
        chain_plans(name, network, "eth0").unwrap()
    }

    fn selected_plan<'a>(plans: &'a [ChainPlan], table: &str, builtin: &str) -> &'a ChainPlan {
        plans
            .iter()
            .find(|plan| plan.table == table && plan.builtin == builtin)
            .unwrap()
    }

    fn rendered_rules(plan: &ChainPlan) -> String {
        plan.rules
            .iter()
            .map(|rule| rule.join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn default_network_is_deterministic() {
        let first = default_network("default", vec![]);
        let replay = default_network("default", vec![]);
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
    }

    #[test]
    fn tap_name_fits_linux_limit() {
        assert!(default_network("default", vec![]).tap_name.len() <= 15);
    }

    #[test]
    fn per_machine_chain_names_are_deterministic() {
        let first = chain_bindings("machine")
            .into_iter()
            .map(|plan| plan.owned)
            .collect::<Vec<_>>();
        let replay = chain_bindings("machine")
            .into_iter()
            .map(|plan| plan.owned)
            .collect::<Vec<_>>();
        assert_eq!(first, replay);
    }

    #[test]
    fn per_machine_chain_names_fit_iptables_limit() {
        assert!(chain_bindings("machine")
            .iter()
            .all(|plan| plan.owned.len() <= 28));
    }

    #[test]
    fn per_machine_chain_plans_are_isolated() {
        let first = plans("first", &default_network("first", vec![]));
        let second = plans("second", &default_network("second", vec![]));
        for left in &first {
            for right in &second {
                assert_ne!(left.owned, right.owned);
                assert_ne!(left.jump_comment, right.jump_comment);
            }
        }
    }

    #[test]
    fn plans_cover_required_hooks() {
        let hooks = chain_bindings("machine")
            .into_iter()
            .map(|plan| (plan.table, plan.builtin))
            .collect::<Vec<_>>();
        assert_eq!(
            hooks,
            vec![
                ("filter", "INPUT"),
                ("filter", "OUTPUT"),
                ("filter", "FORWARD"),
                ("nat", "PREROUTING"),
                ("nat", "OUTPUT"),
                ("nat", "POSTROUTING"),
            ]
        );
    }

    #[test]
    fn rule_plan_generation_is_idempotent() {
        let network = published_network("published");
        assert_eq!(plans("published", &network), plans("published", &network));
    }

    #[test]
    fn published_external_dnat_rule_is_generated() {
        let network = published_network("published");
        let plans = plans("published", &network);
        let rendered = rendered_rules(selected_plan(&plans, "nat", "PREROUTING"));
        assert!(rendered.contains("-p tcp --dport 18080"));
        assert!(rendered.contains("-j DNAT --to-destination"));
        assert!(rendered.contains(&format!("{}:8080", network.guest_address)));
    }

    #[test]
    fn published_localhost_output_dnat_rule_is_generated() {
        let network = published_network("published");
        let plans = plans("published", &network);
        let rendered = rendered_rules(selected_plan(&plans, "nat", "OUTPUT"));
        assert!(rendered.contains("-m addrtype --dst-type LOCAL"));
        assert!(rendered.contains("--dport 18080"));
        assert!(rendered.contains("-j DNAT --to-destination"));
    }

    #[test]
    fn loopback_hairpin_snat_rule_is_generated() {
        let network = published_network("published");
        let plans = plans("published", &network);
        let rendered = rendered_rules(selected_plan(&plans, "nat", "POSTROUTING"));
        assert!(rendered.contains("-s 127.0.0.0/8"));
        assert!(rendered.contains("-j SNAT --to-source"));
        assert!(rendered.contains(&network.gateway_address));
    }

    #[test]
    fn guest_subnet_masquerade_rule_is_generated() {
        let network = default_network("machine", vec![]);
        let plans = plans("machine", &network);
        let rendered = rendered_rules(selected_plan(&plans, "nat", "POSTROUTING"));
        assert!(rendered.contains(&format!("-s {}", subnet_cidr(&network))));
        assert!(rendered.contains("-o eth0"));
        assert!(rendered.contains("-j MASQUERADE"));
    }

    #[test]
    fn guest_forwarding_rule_is_generated() {
        let network = default_network("machine", vec![]);
        let plans = plans("machine", &network);
        let rendered = rendered_rules(selected_plan(&plans, "filter", "FORWARD"));
        assert!(rendered.contains(&format!(
            "-i {} -s {}",
            network.tap_name,
            subnet_cidr(&network)
        )));
        assert!(rendered.contains("-j ACCEPT"));
    }

    #[test]
    fn guest_return_path_rule_is_generated() {
        let network = default_network("machine", vec![]);
        let plans = plans("machine", &network);
        let rendered = rendered_rules(selected_plan(&plans, "filter", "FORWARD"));
        assert!(rendered.contains(&format!(
            "-o {} -d {}",
            network.tap_name,
            subnet_cidr(&network)
        )));
        assert!(rendered.contains("--ctstate ESTABLISHED,RELATED"));
    }

    #[test]
    fn cleanup_bindings_match_installed_chain_identities() {
        let network = published_network("published");
        let installed = plans("published", &network);
        let cleanup = chain_bindings("published");
        assert_eq!(installed.len(), cleanup.len());
        for (installed, cleanup) in installed.iter().zip(cleanup.iter()) {
            assert_eq!(installed.table, cleanup.table);
            assert_eq!(installed.builtin, cleanup.builtin);
            assert_eq!(installed.owned, cleanup.owned);
            assert_eq!(installed.jump_comment, cleanup.jump_comment);
            assert_eq!(jump_rule(installed), jump_rule(cleanup));
        }
    }

    #[test]
    fn duplicate_host_ports_are_rejected() {
        let duplicate = default_network(
            "duplicate",
            vec![
                PublishedPort {
                    protocol: "tcp".to_owned(),
                    host_port: 18080,
                    guest_port: 8080,
                },
                PublishedPort {
                    protocol: "tcp".to_owned(),
                    host_port: 18080,
                    guest_port: 8081,
                },
            ],
        );
        assert!(chain_plans("duplicate", &duplicate, "eth0").is_err());
    }

    #[test]
    fn unsupported_protocols_are_rejected() {
        let unsupported = default_network(
            "unsupported",
            vec![PublishedPort {
                protocol: "sctp".to_owned(),
                host_port: 18080,
                guest_port: 8080,
            }],
        );
        assert!(chain_plans("unsupported", &unsupported, "eth0").is_err());
    }

    #[test]
    fn separate_machines_receive_separate_networks() {
        let first = default_network("first", vec![]);
        let second = default_network("second", vec![]);
        assert_ne!(first.tap_name, second.tap_name);
        assert_ne!(first.guest_mac, second.guest_mac);
        assert_ne!(first.guest_address, second.guest_address);
        assert_ne!(first.gateway_address, second.gateway_address);
    }

    #[test]
    fn rule_comments_are_stable_and_machine_scoped() {
        let name = "published";
        let suffix = chain_suffix(name);
        let plans = plans(name, &published_network(name));
        for plan in &plans {
            assert!(plan.jump_comment.starts_with(&format!("smp:{suffix}:")));
            for rule in &plan.rules {
                let comment = rule
                    .windows(2)
                    .find(|window| window[0] == "--comment")
                    .map(|window| window[1].as_str())
                    .unwrap();
                assert!(comment.starts_with(&format!("smp:{suffix}:")));
            }
        }
    }

    #[test]
    fn distinct_machine_names_have_no_chain_collision() {
        let first = chain_bindings("first")
            .into_iter()
            .map(|plan| plan.owned)
            .collect::<BTreeSet<_>>();
        let second = chain_bindings("second")
            .into_iter()
            .map(|plan| plan.owned)
            .collect::<BTreeSet<_>>();
        assert!(first.is_disjoint(&second));
    }
}
