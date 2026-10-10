use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(super) fn connection_urls(listen: SocketAddr, token: &str) -> Vec<String> {
    let interfaces = if listen.ip() == IpAddr::V4(Ipv4Addr::UNSPECIFIED) {
        match if_addrs::get_if_addrs() {
            Ok(interfaces) => interfaces,
            Err(error) => {
                tracing::warn!(%error, "无法枚举本机网卡地址,仅显示回环访问链接");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    urls_for_ips(
        listen,
        token,
        interfaces
            .into_iter()
            .filter(|iface| iface.is_oper_up())
            .map(|iface| iface.ip()),
    )
}

fn urls_for_ips(
    listen: SocketAddr,
    token: &str,
    ips: impl IntoIterator<Item = IpAddr>,
) -> Vec<String> {
    if !listen.ip().is_unspecified() {
        return vec![format!("http://{listen}/#token={token}")];
    }

    // 保留本机访问链接在首位;0.0.0.0 只监听 IPv4,不能把 IPv6 网卡地址当成可用链接。
    let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut addresses = vec![SocketAddr::new(localhost, listen.port())];
    if listen.is_ipv4() {
        let ips: BTreeSet<_> = ips
            .into_iter()
            .filter(|ip| {
                matches!(ip, IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast())
            })
            .filter(|ip| *ip != localhost)
            .collect();
        addresses.extend(ips.into_iter().map(|ip| SocketAddr::new(ip, listen.port())));
    }
    addresses
        .into_iter()
        .map(|addr| format!("http://{addr}/#token={token}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_lists_all_ipv4_addresses_with_localhost_first() {
        let ips = [
            "192.168.1.20",
            "100.64.0.2",
            "10.0.0.5",
            "192.168.1.20",
            "127.0.0.1",
            "169.254.1.2",
            "::1",
            "2001:db8::1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
        ]
        .map(|ip| ip.parse().unwrap());
        assert_eq!(
            urls_for_ips("0.0.0.0:9000".parse().unwrap(), "tok", ips),
            [
                "http://127.0.0.1:9000/#token=tok",
                "http://10.0.0.5:9000/#token=tok",
                "http://100.64.0.2:9000/#token=tok",
                "http://169.254.1.2:9000/#token=tok",
                "http://192.168.1.20:9000/#token=tok",
            ]
        );
    }

    #[test]
    fn missing_interfaces_still_provides_localhost() {
        assert_eq!(
            urls_for_ips("0.0.0.0:7683".parse().unwrap(), "tok", []),
            ["http://127.0.0.1:7683/#token=tok"]
        );
    }

    #[test]
    fn explicit_bind_only_lists_the_bound_address() {
        for host in [
            "127.0.0.1:7683",
            "192.168.1.20:9000",
            "[::1]:7683",
            "[2001:db8::1]:9000",
        ] {
            assert_eq!(
                urls_for_ips(host.parse().unwrap(), "tok", ["10.0.0.5".parse().unwrap()]),
                [format!("http://{host}/#token=tok")]
            );
        }
    }

    #[tokio::test]
    async fn ephemeral_port_uses_the_actual_listener_port() {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        assert_ne!(listen.port(), 0);
        assert_eq!(
            urls_for_ips(listen, "tok", []),
            [format!("http://127.0.0.1:{}/#token=tok", listen.port())]
        );
    }
}
