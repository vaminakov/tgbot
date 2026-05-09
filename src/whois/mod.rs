use reqwest::Client;
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;
use tracing::info;

use crate::error::BotError;
use crate::i18n::Lang;

/// Query RDAP for an IP address and return a short human-readable summary.
pub async fn lookup(ip: &str, lang: Lang) -> Result<String, BotError> {
    let ip = ip.trim();
    if ip.is_empty() {
        return Ok(lang.whois_usage().to_string());
    }

    let addr: IpAddr = match ip.parse() {
        Ok(a) => a,
        Err(_) => return Ok(lang.whois_not_ip(ip)),
    };

    if is_special(addr) {
        return Ok(format!("🌐 {} — {}", addr, lang.whois_private()));
    }

    let client = Client::builder()
        .user_agent("tgbot/1.0")
        .build()
        .map_err(|e| BotError::Whois {
            message: e.to_string(),
        })?;

    info!(%addr, "RDAP lookup");

    // Try ARIN first (handles redirects to RIPE/APNIC/LACNIC/AFRINIC for non-ARIN IPs).
    // Fall back to RIPE and APNIC directly if ARIN is unreachable.
    let rdap_candidates = [
        format!("https://rdap.arin.net/registry/ip/{}", addr),
        format!("https://rdap.db.ripe.net/ip/{}", addr),
        format!("https://rdap.apnic.net/ip/{}", addr),
    ];

    let mut rdap_data: Option<Value> = None;
    let mut rdap_err = String::new();

    for candidate in &rdap_candidates {
        match client
            .get(candidate)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                Ok(data) => {
                    rdap_data = Some(data);
                    break;
                }
                Err(e) => {
                    rdap_err = e.to_string();
                }
            },
            Ok(resp) => {
                rdap_err = format!("HTTP {}", resp.status());
            }
            Err(e) => {
                rdap_err = e.to_string();
            }
        }
    }

    let data = rdap_data.ok_or_else(|| BotError::Whois { message: rdap_err })?;

    Ok(format_rdap(addr, &data, lang))
}

fn is_special(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

fn format_rdap(addr: IpAddr, data: &Value, lang: Lang) -> String {
    let mut lines = vec![format!("🌐 {}", addr)];

    // Network name and address range
    let name = data["name"].as_str().unwrap_or("—");
    let start = data["startAddress"].as_str().unwrap_or("");
    let end = data["endAddress"].as_str().unwrap_or("");
    if !start.is_empty() && !end.is_empty() && start != end {
        lines.push(format!(
            "🔢 {}: {}  ({} — {})",
            lang.whois_network(),
            name,
            start,
            end
        ));
    } else {
        lines.push(format!("🔢 {}: {}", lang.whois_network(), name));
    }

    // Country
    if let Some(country) = data["country"].as_str() {
        lines.push(format!("🌍 {}: {}", lang.whois_country(), country));
    }

    // Organisation details (registrant entity)
    if let Some(entities) = data["entities"].as_array() {
        if let Some(org) = find_vcard_field_by_role(entities, "registrant", "fn") {
            lines.push(format!("🏢 {}: {}", lang.whois_org(), org));
        }
        // City/region from registrant postal address (org location, not IP geolocation)
        if let Some(city) = find_vcard_adr_city_by_role(entities, "registrant") {
            lines.push(format!("📍 {}: {}", lang.whois_city(), city));
        }
        // Registrant contact email (may differ from abuse address)
        if let Some(email) = find_vcard_field_by_role(entities, "registrant", "email") {
            lines.push(format!("✉️ {}: {}", lang.whois_contact(), email));
        }
        // Phone
        if let Some(phone) = find_vcard_phone_by_role(entities, "registrant") {
            lines.push(format!("📞 {}: {}", lang.whois_phone(), phone));
        }
        // Abuse contact
        if let Some(email) = find_vcard_field_by_role(entities, "abuse", "email") {
            lines.push(format!("📧 {}: {}", lang.whois_abuse(), email));
        }
    }

    lines.join("\n")
}

/// Recursively search entities for one with the given role, then return the
/// value of the requested vCard field (e.g. "fn", "email").
fn find_vcard_field_by_role(entities: &[Value], role: &str, field: &str) -> Option<String> {
    for entity in entities {
        if has_role(entity, role) {
            if let Some(val) = get_vcard_field(entity, field) {
                return Some(val);
            }
        }
        // Recurse into nested entities (e.g. abuse contact inside registrant)
        if let Some(nested) = entity["entities"].as_array() {
            if let Some(val) = find_vcard_field_by_role(nested, role, field) {
                return Some(val);
            }
        }
    }
    None
}

/// Extract city (and optionally region) from registrant vCard `adr` field.
/// RDAP structured adr value: ["", pobox, ext, street, city, region, zip, country]
fn find_vcard_adr_city_by_role(entities: &[Value], role: &str) -> Option<String> {
    for entity in entities {
        if has_role(entity, role) {
            if let Some(city) = get_vcard_adr_city(entity) {
                return Some(city);
            }
        }
        if let Some(nested) = entity["entities"].as_array() {
            if let Some(city) = find_vcard_adr_city_by_role(nested, role) {
                return Some(city);
            }
        }
    }
    None
}

fn get_vcard_adr_city(entity: &Value) -> Option<String> {
    let vcard = entity["vcardArray"].as_array()?;
    let props = vcard.get(1)?.as_array()?;
    for prop in props {
        let prop = prop.as_array()?;
        if prop.first()?.as_str() != Some("adr") {
            continue;
        }
        // Structured value is an array: ["", pobox, ext, street, city, region, zip, country]
        if let Some(parts) = prop.get(3).and_then(|v| v.as_array()) {
            let city = parts.get(4).and_then(|v| v.as_str()).unwrap_or("").trim();
            let region = parts.get(5).and_then(|v| v.as_str()).unwrap_or("").trim();
            let result = match (city.is_empty(), region.is_empty()) {
                (false, false) => format!("{}, {}", city, region),
                (false, true) => city.to_string(),
                (true, false) => region.to_string(),
                (true, true) => return None,
            };
            return Some(result);
        }
        // Fallback: label text
        if let Some(label) = prop.get(3).and_then(|v| v.as_str()) {
            let first_line = label.lines().next().unwrap_or("").trim();
            if !first_line.is_empty() {
                return Some(first_line.to_string());
            }
        }
    }
    None
}

/// Extract phone from vCard `tel` field, stripping the "tel:" URI prefix.
fn find_vcard_phone_by_role(entities: &[Value], role: &str) -> Option<String> {
    for entity in entities {
        if has_role(entity, role) {
            if let Some(tel) = get_vcard_field(entity, "tel") {
                let cleaned = tel.trim_start_matches("tel:").to_string();
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        }
        if let Some(nested) = entity["entities"].as_array() {
            if let Some(tel) = find_vcard_phone_by_role(nested, role) {
                return Some(tel);
            }
        }
    }
    None
}

fn has_role(entity: &Value, role: &str) -> bool {
    entity["roles"]
        .as_array()
        .map(|roles| roles.iter().any(|r| r.as_str() == Some(role)))
        .unwrap_or(false)
}

/// Extract a field value from an entity's vcardArray.
/// vcardArray = ["vcard", [[type, params, kind, value], ...]]
fn get_vcard_field(entity: &Value, field_type: &str) -> Option<String> {
    let vcard = entity["vcardArray"].as_array()?;
    let props = vcard.get(1)?.as_array()?;
    for prop in props {
        let prop = prop.as_array()?;
        if prop.first()?.as_str() == Some(field_type) {
            return prop.get(3).and_then(|v| v.as_str()).map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── is_special ──────────────────────────────────────────────────────────

    #[test]
    fn special_private_ipv4() {
        assert!(is_special("10.0.0.1".parse().unwrap()));
        assert!(is_special("192.168.1.1".parse().unwrap()));
        assert!(is_special("172.16.0.1".parse().unwrap()));
    }

    #[test]
    fn special_loopback_ipv4() {
        assert!(is_special("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn special_link_local_ipv4() {
        assert!(is_special("169.254.1.1".parse().unwrap()));
    }

    #[test]
    fn special_broadcast_ipv4() {
        assert!(is_special("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn public_ipv4_not_special() {
        assert!(!is_special("8.8.8.8".parse().unwrap()));
        assert!(!is_special("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn special_ipv6_loopback() {
        assert!(is_special("::1".parse().unwrap()));
    }

    #[test]
    fn public_ipv6_not_special() {
        assert!(!is_special("2001:db8::1".parse().unwrap()));
    }

    // ── format_rdap ─────────────────────────────────────────────────────────

    #[test]
    fn format_rdap_includes_ip_name_country() {
        let addr: IpAddr = "8.8.8.8".parse().unwrap();
        let data = json!({
            "name": "GOGL",
            "startAddress": "8.8.8.0",
            "endAddress": "8.8.8.255",
            "country": "US"
        });
        let result = format_rdap(addr, &data, crate::i18n::Lang::En);
        assert!(result.contains("8.8.8.8"));
        assert!(result.contains("GOGL"));
        assert!(result.contains("US"));
        assert!(result.contains("8.8.8.0"));
        assert!(result.contains("8.8.8.255"));
    }

    #[test]
    fn format_rdap_start_equals_end_no_range_shown() {
        let addr: IpAddr = "8.8.8.8".parse().unwrap();
        let data = json!({ "name": "X", "startAddress": "8.8.8.8", "endAddress": "8.8.8.8" });
        let result = format_rdap(addr, &data, crate::i18n::Lang::En);
        assert!(!result.contains(" — "), "should not display range when start == end");
    }

    #[test]
    fn format_rdap_with_registrant_org() {
        let addr: IpAddr = "1.1.1.1".parse().unwrap();
        let data = json!({
            "name": "APNIC",
            "entities": [{
                "roles": ["registrant"],
                "vcardArray": ["vcard", [
                    ["fn", {}, "text", "Cloudflare Inc"]
                ]]
            }]
        });
        let result = format_rdap(addr, &data, crate::i18n::Lang::En);
        assert!(result.contains("Cloudflare Inc"));
    }

    // ── get_vcard_adr_city ───────────────────────────────────────────────────

    #[test]
    fn vcard_adr_city_and_region() {
        let entity = json!({
            "vcardArray": ["vcard", [
                ["adr", {}, "text", ["", "", "", "", "Amsterdam", "NH", "1000", "NL"]]
            ]]
        });
        assert_eq!(get_vcard_adr_city(&entity).unwrap(), "Amsterdam, NH");
    }

    #[test]
    fn vcard_adr_city_only() {
        let entity = json!({
            "vcardArray": ["vcard", [
                ["adr", {}, "text", ["", "", "", "", "Berlin", "", "", "DE"]]
            ]]
        });
        assert_eq!(get_vcard_adr_city(&entity).unwrap(), "Berlin");
    }

    #[test]
    fn vcard_adr_region_only() {
        let entity = json!({
            "vcardArray": ["vcard", [
                ["adr", {}, "text", ["", "", "", "", "", "Bavaria", "", "DE"]]
            ]]
        });
        assert_eq!(get_vcard_adr_city(&entity).unwrap(), "Bavaria");
    }

    #[test]
    fn vcard_adr_both_empty_is_none() {
        let entity = json!({
            "vcardArray": ["vcard", [
                ["adr", {}, "text", ["", "", "", "", "", "", "", ""]]
            ]]
        });
        assert!(get_vcard_adr_city(&entity).is_none());
    }

    // ── find_vcard_field_by_role (nested recursion) ──────────────────────────

    #[test]
    fn find_vcard_field_in_nested_entity() {
        let entities = json!([{
            "roles": ["registrant"],
            "entities": [{
                "roles": ["abuse"],
                "vcardArray": ["vcard", [
                    ["email", {}, "text", "abuse@example.com"]
                ]]
            }]
        }]);
        let result =
            find_vcard_field_by_role(entities.as_array().unwrap(), "abuse", "email");
        assert_eq!(result.unwrap(), "abuse@example.com");
    }

    // ── find_vcard_phone_by_role ─────────────────────────────────────────────

    #[test]
    fn phone_tel_uri_prefix_stripped() {
        let entities = json!([{
            "roles": ["registrant"],
            "vcardArray": ["vcard", [
                ["tel", {}, "uri", "tel:+1-800-555-0100"]
            ]]
        }]);
        let result =
            find_vcard_phone_by_role(entities.as_array().unwrap(), "registrant");
        assert_eq!(result.unwrap(), "+1-800-555-0100");
    }
}
