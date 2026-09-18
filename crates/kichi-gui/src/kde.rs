//! 读取 KDE 系统配色(kdeglobals), 让界面跟随系统的 Breeze 主题与强调色。
//! 仅做只读解析, 不依赖 KDE/Qt 运行库。

use std::collections::HashMap;

use eframe::egui::Color32;

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn color(self) -> Color32 {
        Color32::from_rgb(self.0, self.1, self.2)
    }

    fn luma(self) -> f32 {
        0.2126 * self.0 as f32 + 0.7152 * self.1 as f32 + 0.0722 * self.2 as f32
    }
}

/// 从系统配色方案中提取的关键颜色。
#[derive(Clone, Debug)]
pub struct KdeColors {
    pub dark: bool,
    pub window_bg: Color32,
    pub window_fg: Color32,
    pub view_bg: Color32,
    pub accent: Color32,
    /// 强调色上的前景色(通常为黑或白)。
    pub on_accent: Color32,
    /// 次级文字(失效态)。
    pub inactive: Color32,
    pub positive: Color32,
    pub neutral: Color32,
    pub negative: Color32,
}

fn parse_rgb(v: &str) -> Option<Rgb> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Rgb(r, g, b));
        }
    }
    let mut it = v.split(',');
    let r: u8 = it.next()?.trim().parse().ok()?;
    let g: u8 = it.next()?.trim().parse().ok()?;
    let b: u8 = it.next()?.trim().parse().ok()?;
    Some(Rgb(r, g, b))
}

fn parse(text: &str) -> Option<KdeColors> {
    let mut group = String::new();
    let mut map: HashMap<String, Rgb> = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            group = rest.split(']').next().unwrap_or("").to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some(rgb) = parse_rgb(v) {
                map.insert(format!("{group}/{k}"), rgb);
            }
        }
    }
    let get = |g: &str, k: &str| map.get(&format!("{g}/{k}")).copied();

    let window_bg = get("Colors:Window", "BackgroundNormal")?;
    let window_fg = get("Colors:Window", "ForegroundNormal").unwrap_or(Rgb(35, 38, 41));
    let view_bg = get("Colors:View", "BackgroundNormal").unwrap_or(window_bg);
    let selection_bg = get("Colors:Selection", "BackgroundNormal").unwrap_or(window_fg);
    let on_accent = get("Colors:Selection", "ForegroundNormal").unwrap_or(window_bg);
    // Plasma 5.24+ 在 [General] 里直接给出强调色; 旧版本回退到选择色/焦点色。
    let accent = get("General", "AccentColor")
        .or_else(|| get("Colors:Window", "DecorationFocus"))
        .unwrap_or(selection_bg);
    let inactive = get("Colors:Window", "ForegroundInactive").unwrap_or(window_fg);
    let positive = get("Colors:Window", "ForegroundPositive").unwrap_or(Rgb(39, 174, 96));
    let neutral = get("Colors:Window", "ForegroundNeutral").unwrap_or(Rgb(246, 116, 0));
    let negative = get("Colors:Window", "ForegroundNegative").unwrap_or(Rgb(218, 68, 83));

    Some(KdeColors {
        dark: window_bg.luma() < 128.0,
        window_bg: window_bg.color(),
        window_fg: window_fg.color(),
        view_bg: view_bg.color(),
        accent: accent.color(),
        on_accent: on_accent.color(),
        inactive: inactive.color(),
        positive: positive.color(),
        neutral: neutral.color(),
        negative: negative.color(),
    })
}

fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("kdeglobals"))
}

/// 读取系统配色; 非 KDE 系统或读取失败时返回 `None`。
pub fn load() -> Option<KdeColors> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::parse;

    const SAMPLE: &str = "\
[Colors:View]
BackgroundNormal=255,255,255
ForegroundNormal=35,38,41

[Colors:Window]
BackgroundNormal=239,240,241
ForegroundNormal=35,38,41
ForegroundInactive=112,125,138
ForegroundPositive=39,174,96
ForegroundNeutral=246,116,0
ForegroundNegative=218,68,83

[Colors:Selection]
BackgroundNormal=61,174,233
ForegroundNormal=255,255,255

[General]
AccentColor=61,174,233
";

    #[test]
    fn parses_light_breeze() {
        let c = parse(SAMPLE).expect("parse");
        assert!(!c.dark);
        assert_eq!(c.accent, eframe::egui::Color32::from_rgb(61, 174, 233));
        assert_eq!(c.on_accent, eframe::egui::Color32::from_rgb(255, 255, 255));
        assert_eq!(c.view_bg, eframe::egui::Color32::from_rgb(255, 255, 255));
    }

    #[test]
    fn detects_dark_scheme() {
        let dark = SAMPLE.replace("BackgroundNormal=239,240,241", "BackgroundNormal=27,30,32");
        let c = parse(&dark).expect("parse");
        assert!(c.dark);
    }
}
