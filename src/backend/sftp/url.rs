use std::io;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SftpUrl {
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
}
impl SftpUrl {
    pub fn parse(url: &str) -> io::Result<Self> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "invalid SFTP repository URL");
        let (authority, path, uri) = if let Some(rest) = url.strip_prefix("sftp://") {
            let (authority, path) = rest.split_once('/').ok_or_else(bad)?;
            (authority, format!("/{path}"), true)
        } else {
            let rest = url.strip_prefix("sftp:").ok_or_else(bad)?;
            let split = rest.find(":/").ok_or_else(bad)?;
            (&rest[..split], rest[split + 1..].to_owned(), false)
        };
        let host_start = authority.rfind('@').map_or(0, |i| i + 1);
        let address = &authority[host_start..];
        let (host, port) = if address.starts_with('[') {
            let end = address.find(']').ok_or_else(bad)?;
            let suffix = &address[end + 1..];
            let port = if suffix.is_empty() {
                None
            } else if uri {
                Some(
                    suffix
                        .strip_prefix(':')
                        .ok_or_else(bad)?
                        .parse::<u16>()
                        .map_err(|_| bad())?,
                )
            } else {
                return Err(bad());
            };
            (
                format!("{}{}", &authority[..host_start], &address[1..end]),
                port,
            )
        } else if uri {
            let (address, port) = match address.split_once(':') {
                Some((a, p)) => (a, Some(p.parse::<u16>().map_err(|_| bad())?)),
                None => (address, None),
            };
            (format!("{}{address}", &authority[..host_start]), port)
        } else {
            (authority.to_owned(), None)
        };
        if host.rsplit('@').next().is_none_or(str::is_empty)
            || authority.bytes().filter(|b| *b == b'@').count() > 1
            || host.is_empty()
            || address.is_empty()
            || address.starts_with('-')
            || host.starts_with('-')
            || host
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '/')
            || path.contains('\0')
            || port == Some(0)
            || (host_start > 0 && host_start == 1)
        {
            return Err(bad());
        }
        Ok(Self { host, port, path })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sftp_url_table() {
        for (url, host, port, path) in [
            ("sftp:alias:/repo", "alias", None, "/repo"),
            ("sftp:u@host:/a b", "u@host", None, "/a b"),
            ("sftp://u@host:2222/repo", "u@host", Some(2222), "/repo"),
            ("sftp:[::1]:/repo", "::1", None, "/repo"),
            ("sftp://u@[::1]:22/repo", "u@::1", Some(22), "/repo"),
        ] {
            assert_eq!(
                SftpUrl::parse(url).unwrap(),
                SftpUrl {
                    host: host.into(),
                    port,
                    path: path.into()
                }
            );
        }
        for url in [
            "sftp:x",
            "sftp://x:0/r",
            "sftp://x:no/r",
            "sftp:-oProxyCommand=x:/r",
            "sftp://[::1/r",
            "sftp:@x:/r",
            "sftp://u@:22/r",
            "sftp:u@[]:/r",
            "sftp:a@@host:/r",
            "sftp:host\t:/r",
        ] {
            assert!(SftpUrl::parse(url).is_err(), "{url}");
        }
    }
}
