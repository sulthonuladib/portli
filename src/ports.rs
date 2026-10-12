//! Pure port-list parsing and well-known port metadata.
//!
//! This module has no terminal or network dependencies so the parsing rules can
//! be exercised in isolation.

use std::fmt;

/// Ports shown when the user leaves the input empty.
pub const DEFAULT_PORTS: [u16; 10] = [21, 22, 23, 25, 53, 80, 110, 143, 443, 3306];

/// Upper bound on how many ports a single scan may target.
///
/// This keeps an accidental `1-65535` from spawning a multi-minute scan.
pub const MAX_SCAN_PORTS: usize = 1024;

/// Errors produced while parsing a user supplied port expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The expression contained no ports at all.
    Empty,
    /// A token was not a number.
    InvalidNumber { token: String },
    /// A number did not fit in a `u16`.
    OutOfRange { token: String },
    /// A range like `90-80` where the start is greater than the end.
    InvertedRange { start: u16, end: u16 },
    /// The expression expanded to more than [`MAX_SCAN_PORTS`].
    TooManyPorts { count: usize, max: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "no ports specified"),
            Self::InvalidNumber { token } => write!(f, "`{token}` is not a port number"),
            Self::OutOfRange { token } => write!(f, "`{token}` is outside 0-65535"),
            Self::InvertedRange { start, end } => {
                write!(
                    f,
                    "invalid range `{start}-{end}` (start is greater than end)"
                )
            }
            Self::TooManyPorts { count, max } => {
                write!(f, "{count} ports requested, but the limit is {max}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a comma/space/semicolon separated list of ports and ranges.
///
/// Accepts expressions such as `22`, `22,80,443`, `8000-8010` and
/// `22 80 443`. The result is sorted and de-duplicated. An empty (or
/// whitespace-only) input is an error; callers that want a default should
/// check for emptiness themselves.
pub fn parse_ports(input: &str) -> Result<Vec<u16>, ParseError> {
    let mut ports: Vec<u16> = Vec::new();

    for token in input.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        if let Some((start, end)) = token.split_once('-') {
            let start = parse_port(start)?;
            let end = parse_port(end)?;
            if start > end {
                return Err(ParseError::InvertedRange { start, end });
            }
            let count = ports.len() + (end as usize - start as usize + 1);
            if count > MAX_SCAN_PORTS {
                return Err(ParseError::TooManyPorts {
                    count,
                    max: MAX_SCAN_PORTS,
                });
            }
            ports.extend(start..=end);
        } else {
            if ports.len() >= MAX_SCAN_PORTS {
                return Err(ParseError::TooManyPorts {
                    count: ports.len() + 1,
                    max: MAX_SCAN_PORTS,
                });
            }
            ports.push(parse_port(token)?);
        }
    }

    if ports.is_empty() {
        return Err(ParseError::Empty);
    }

    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

fn parse_port(token: &str) -> Result<u16, ParseError> {
    let token = token.trim();
    let value: u32 = token.parse().map_err(|_| ParseError::InvalidNumber {
        token: token.to_owned(),
    })?;
    u16::try_from(value).map_err(|_| ParseError::OutOfRange {
        token: token.to_owned(),
    })
}

/// Best-effort service name for a well-known TCP port.
pub fn service_name(port: u16) -> Option<&'static str> {
    Some(match port {
        20 => "ftp-data",
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "domain",
        67 | 68 => "dhcp",
        69 => "tftp",
        80 => "http",
        110 => "pop3",
        123 => "ntp",
        135 => "msrpc",
        137..=139 => "netbios",
        143 => "imap",
        161 => "snmp",
        162 => "snmptrap",
        179 => "bgp",
        389 => "ldap",
        443 => "https",
        445 => "microsoft-ds",
        465 => "smtps",
        514 => "syslog",
        587 => "submission",
        631 => "ipp",
        636 => "ldaps",
        873 => "rsync",
        993 => "imaps",
        995 => "pop3s",
        1080 => "socks",
        1433 => "mssql",
        1521 => "oracle",
        1723 => "pptp",
        2049 => "nfs",
        2375 => "docker",
        2376 => "docker-tls",
        3000 => "dev-http",
        3306 => "mysql",
        3389 => "rdp",
        4443 => "https-alt",
        5000 => "upnp",
        5432 => "postgres",
        5601 => "kibana",
        5672 => "amqp",
        5900 => "vnc",
        6379 => "redis",
        6443 => "kubernetes",
        7000 => "cassandra",
        8000 => "http-alt",
        8080 => "http-proxy",
        8081 => "http-alt",
        8443 => "https-alt",
        8888 => "http-alt",
        9000 => "sonarqube",
        9092 => "kafka",
        9200 | 9300 => "elasticsearch",
        11211 => "memcached",
        15672 => "rabbitmq-mgmt",
        27017 | 27018 => "mongodb",
        50000 => "sap",
        50051 => "grpc",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists_and_ranges() {
        assert_eq!(parse_ports("22,80,443").unwrap(), vec![22, 80, 443]);
        assert_eq!(
            parse_ports("8000-8003").unwrap(),
            vec![8000, 8001, 8002, 8003]
        );
        assert_eq!(parse_ports(" 22  80 ").unwrap(), vec![22, 80]);
        assert_eq!(parse_ports("443;22;443").unwrap(), vec![22, 443]);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse_ports(""), Err(ParseError::Empty));
        assert!(matches!(
            parse_ports("abc"),
            Err(ParseError::InvalidNumber { .. })
        ));
        assert!(matches!(
            parse_ports("70000"),
            Err(ParseError::OutOfRange { .. })
        ));
        assert!(matches!(
            parse_ports("90-80"),
            Err(ParseError::InvertedRange { .. })
        ));
        assert!(matches!(
            parse_ports("1-1025"),
            Err(ParseError::TooManyPorts { .. })
        ));
    }

    #[test]
    fn default_ports_are_ten_and_valid() {
        assert_eq!(DEFAULT_PORTS.len(), 10);
        assert!(DEFAULT_PORTS.iter().all(|&p| service_name(p).is_some()));
    }
}
