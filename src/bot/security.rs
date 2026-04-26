use crate::config::IpWhitelistConfig;
use crate::error::BotError;
use ipnet::IpNet;
use std::net::IpAddr;

/// Hardcoded Telegram webhook source IP ranges.
const TELEGRAM_CIDRS: &[&str] = &[
    "149.154.160.0/20",
    "91.108.4.0/22",
    "91.108.8.0/22",
    "91.108.12.0/22",
    "91.108.16.0/22",
    "91.108.56.0/22",
    "91.108.20.0/22",
    "185.76.151.0/24",
    "2001:b28:f23d::/48",
    "2001:b28:f23f::/48",
    "2001:67c:4e8::/48",
];

#[derive(Debug)]
pub enum IpWhitelist {
    Disabled,
    CidrList(Vec<IpNet>),
}

impl IpWhitelist {
    pub fn from_config(cfg: &IpWhitelistConfig) -> Result<Self, BotError> {
        match cfg {
            IpWhitelistConfig::Named(s) if s == "disabled" => Ok(Self::Disabled),
            IpWhitelistConfig::Named(s) if s == "telegram" => {
                let nets: Vec<IpNet> = TELEGRAM_CIDRS
                    .iter()
                    .map(|s| {
                        s.parse::<IpNet>()
                            .expect("hardcoded TELEGRAM_CIDRS are always valid")
                    })
                    .collect();
                Ok(Self::CidrList(nets))
            }
            IpWhitelistConfig::Named(s) => Err(BotError::InvalidArgument {
                input: format!(
                    "unknown webhook_ip_whitelist: '{}'; use \"telegram\", \"disabled\", or a CIDR list",
                    s
                ),
            }),
            IpWhitelistConfig::Custom(cidrs) => {
                let nets: Vec<IpNet> = cidrs
                    .iter()
                    .map(|s| {
                        s.parse::<IpNet>().map_err(|e| BotError::InvalidArgument {
                            input: format!("invalid CIDR '{s}': {e}"),
                        })
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Self::CidrList(nets))
            }
        }
    }

    pub fn allows(&self, ip: IpAddr) -> bool {
        match self {
            Self::Disabled => true,
            Self::CidrList(nets) => nets.iter().any(|net| net.contains(&ip)),
        }
    }
}

/// Validate a command argument for {arg1} placeholder.
/// Allows: a-z A-Z 0-9 . _ / : -
pub fn sanitize_arg(input: &str) -> Result<&str, BotError> {
    if !input.is_empty()
        && input
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/:-".contains(c))
    {
        Ok(input)
    } else {
        Err(BotError::InvalidArgument {
            input: input.to_string(),
        })
    }
}

/// Expand {arg1} and {args} placeholders in a command template.
pub fn expand_cmd(template: &str, parts: &[&str]) -> String {
    let arg1 = parts.first().copied().unwrap_or("");
    let args = parts.join(" ");
    template.replace("{arg1}", arg1).replace("{args}", &args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IpWhitelistConfig;

    #[test]
    fn test_telegram_ip_allowed() {
        let wl = IpWhitelist::from_config(&IpWhitelistConfig::Named("telegram".into())).unwrap();
        assert!(wl.allows("149.154.160.1".parse().unwrap()));
        assert!(wl.allows("91.108.4.1".parse().unwrap()));
    }

    #[test]
    fn test_random_ip_blocked() {
        let wl = IpWhitelist::from_config(&IpWhitelistConfig::Named("telegram".into())).unwrap();
        assert!(!wl.allows("1.2.3.4".parse().unwrap()));
    }

    #[test]
    fn test_disabled_allows_all() {
        let wl = IpWhitelist::from_config(&IpWhitelistConfig::Named("disabled".into())).unwrap();
        assert!(wl.allows("1.2.3.4".parse().unwrap()));
        assert!(wl.allows("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_custom_cidr() {
        let wl = IpWhitelist::from_config(&IpWhitelistConfig::Custom(vec!["10.0.0.0/8".into()]))
            .unwrap();
        assert!(wl.allows("10.1.2.3".parse().unwrap()));
        assert!(!wl.allows("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_sanitize_valid() {
        assert_eq!(sanitize_arg("192.168.1.1").unwrap(), "192.168.1.1");
        assert_eq!(sanitize_arg("example.com").unwrap(), "example.com");
        assert_eq!(sanitize_arg("geoip_on").unwrap(), "geoip_on");
    }

    #[test]
    fn test_unknown_whitelist_value_is_error() {
        let result = IpWhitelist::from_config(&IpWhitelistConfig::Named("Telegram".into()));
        assert!(result.is_err());
        let result = IpWhitelist::from_config(&IpWhitelistConfig::Named("all".into()));
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_rejects_shell_chars() {
        assert!(sanitize_arg("; rm -rf /").is_err());
        assert!(sanitize_arg("$(evil)").is_err());
        assert!(sanitize_arg("`cmd`").is_err());
        assert!(sanitize_arg("a b").is_err());
        assert!(sanitize_arg("").is_err());
    }

    #[test]
    fn test_sanitize_rejects_non_ascii() {
        assert!(sanitize_arg("café").is_err());
        assert!(sanitize_arg("中文").is_err());
    }

    #[test]
    fn test_expand_arg1() {
        assert_eq!(
            expand_cmd("sudo unban {arg1}", &["1.2.3.4"]),
            "sudo unban 1.2.3.4"
        );
    }

    #[test]
    fn test_expand_args() {
        assert_eq!(
            expand_cmd("sudo snft {args}", &["-d", "-f", "20"]),
            "sudo snft -d -f 20"
        );
    }
}
