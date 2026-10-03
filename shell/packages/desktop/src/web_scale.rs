//! Pixel placements and browser zoom share the interface scale.
use crate::common::LengthValue;
pub fn percent(value: &serde_json::Value) -> u32 {
    match value.get("uiScale").and_then(|v| v.as_u64()) {
        Some(n @ (85 | 90 | 100 | 110 | 125 | 150)) => n as u32,
        _ => 100,
    }
}
pub fn factor() -> f32 {
    let value = std::env::var_os("USERPROFILE")
        .and_then(|home| std::fs::read(std::path::PathBuf::from(home).join(".config/logical-lunge/prefs.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()).unwrap_or_default();
    percent(&value) as f32 / 100.0
}
pub fn length(value: &LengthValue, total: i32, dpi: f32, scale: f32) -> i32 {
    value.to_px_scaled(total, dpi * scale)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_scale_falls_back_without_changing_screen_percentages() {
        assert_eq!(percent(&serde_json::json!({"uiScale": -1})), 100);
        assert_eq!(percent(&serde_json::json!({"uiScale": 149})), 100);
        assert_eq!(percent(&serde_json::json!({"uiScale": 150})), 150);
        assert_eq!(length(&"100%".parse().unwrap(), 1920, 1.25, 1.5), 1920);
        assert_eq!(length(&"40px".parse().unwrap(), 1080, 1.25, 1.5), 75);
    }
}
