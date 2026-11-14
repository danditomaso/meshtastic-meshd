//! Name and node-id parsing, matching the Android app's rules.

use std::sync::LazyLock;

use regex::Regex;

/// BLE names end in the node id's last 4 hex digits, e.g. `Meshtastic_1a2b`.
static BLE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^.*_([0-9a-fA-F]{4})$").expect("valid regex"));

/// Used when a radio advertises no short name.
pub const DEFAULT_SHORT_NAME: &str = "Meshtastic";

/// `Meshtastic_1A2b` → `1a2b`.
pub fn ble_name_suffix(name: &str) -> Option<String> {
    BLE_NAME
        .captures(name)
        .map(|caps| caps[1].to_ascii_lowercase())
}

/// `A1B2C3D4` or `!a1b2c3d4` → `!a1b2c3d4`; `None` if not hex.
pub fn normalize_node_id(raw: &str) -> Option<String> {
    let hex = raw.trim().trim_start_matches('!');
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("!{}", hex.to_ascii_lowercase()))
}

/// Last 4 hex digits of a node id.
pub fn node_id_suffix(node_id: &str) -> Option<&str> {
    let hex = node_id.trim_start_matches('!');
    (hex.len() >= 4).then(|| &hex[hex.len() - 4..])
}

/// `<shortname>_<id>`, unless the short name already contains the id (as on Android).
pub fn tcp_display_name(short_name: Option<&str>, node_id: Option<&str>) -> String {
    let mut name = short_name.unwrap_or(DEFAULT_SHORT_NAME).to_owned();
    if let Some(id) = node_id.map(|id| id.trim_start_matches('!'))
        && name.split('_').all(|part| part != id)
    {
        name.push('_');
        name.push_str(id);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_suffix_matches_meshtastic_names() {
        assert_eq!(ble_name_suffix("Meshtastic_1A2b").as_deref(), Some("1a2b"));
        assert_eq!(ble_name_suffix("T-Beam_ab_c0de").as_deref(), Some("c0de"));
    }

    #[test]
    fn ble_suffix_rejects_other_names() {
        assert_eq!(ble_name_suffix("Meshtastic"), None);
        assert_eq!(ble_name_suffix("Meshtastic_12345"), None);
        assert_eq!(ble_name_suffix("Meshtastic_12g4"), None);
        assert_eq!(ble_name_suffix("1a2b"), None);
    }

    #[test]
    fn node_id_is_normalised() {
        assert_eq!(normalize_node_id("!A1B2C3D4").as_deref(), Some("!a1b2c3d4"));
        assert_eq!(normalize_node_id("a1b2c3d4").as_deref(), Some("!a1b2c3d4"));
        assert_eq!(
            normalize_node_id(" !a1b2c3d4 ").as_deref(),
            Some("!a1b2c3d4")
        );
        assert_eq!(normalize_node_id("!"), None);
        assert_eq!(normalize_node_id("node-1"), None);
    }

    #[test]
    fn node_id_suffix_is_last_four() {
        assert_eq!(node_id_suffix("!a1b2c3d4"), Some("c3d4"));
        assert_eq!(node_id_suffix("!abc"), None);
    }

    #[test]
    fn display_name_follows_android_rules() {
        assert_eq!(
            tcp_display_name(Some("ABCD"), Some("!a1b2c3d4")),
            "ABCD_a1b2c3d4"
        );
        assert_eq!(
            tcp_display_name(Some("ABCD_a1b2c3d4"), Some("!a1b2c3d4")),
            "ABCD_a1b2c3d4"
        );
        assert_eq!(
            tcp_display_name(None, Some("!a1b2c3d4")),
            "Meshtastic_a1b2c3d4"
        );
        assert_eq!(tcp_display_name(Some("ABCD"), None), "ABCD");
    }
}
