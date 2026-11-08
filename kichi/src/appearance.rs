//! 系统外观：深色偏好与强调色。
//!
//! 数据来源是 XDG Desktop Portal 的 `org.freedesktop.appearance`：
//! `color-scheme`（0 无偏好 / 1 深色 / 2 浅色）与 `accent-color`（sRGB 0..1）。
//! 这是 GNOME、KDE、Flatpak 共同支持的接口，不依赖宿主是否安装了某个 Qt
//! 平台主题插件；AppImage 的 `qt.conf` 排除宿主插件时也照样可用。
//!
//! 只在 `QPalette` 能完整控制的控件风格下启用（默认是 Fusion）。KDE 的
//! `org.kde.desktop` 由 Kirigami / `kdeglobals` 驱动，这里不覆盖，避免破坏
//! Plasma 原生观感。
//!
//! 调色板必须在 GUI 线程应用。后台线程只轮询并投递 [`Appearance`]，
//! 由 QML 每 100ms 调用的 `Kichi::pump()` 在主线程消费。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

use ashpd::desktop::settings::{ColorScheme, Settings};
use ashpd::zbus::block_on;

extern "C" {
    /// `cpp/app_platform.cpp`：应用/恢复系统调色板。`scheme` 为 -1 表示
    /// 没有明确偏好、沿用 Qt 当前调色板；0 = 浅色，1 = 深色。
    fn kichi_apply_appearance(scheme: i32, has_accent: i32, r: f64, g: f64, b: f64);
}

/// 系统外观偏好。
///
/// `dark == None` 表示桌面没有明确偏好：继续用 Qt 当前调色板，不强行套用
/// 应用自带的浅色 / 深色。`accent` 为 `None` 时保留桌面自己的高亮色。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Appearance {
    pub dark: Option<bool>,
    /// sRGB，每个分量 0..=1；门户给出越界值时按未设置处理。
    pub accent: Option<[f64; 3]>,
}

/// 首次探测最多等这么久，避免门户 D-Bus 卡住启动；超时后线程拿到结果会
/// 通过 `pump()` 补上。
const STARTUP_TIMEOUT: Duration = Duration::from_millis(500);
/// 门户没有信号订阅时，轮询间隔足够短，切换主题几乎即时可见。
const POLL_INTERVAL: Duration = Duration::from_secs(2);

static LATEST: OnceLock<Mutex<Appearance>> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);

fn latest() -> &'static Mutex<Appearance> {
    LATEST.get_or_init(|| Mutex::new(Appearance::default()))
}

fn publish(appearance: Appearance) {
    *latest().lock().unwrap_or_else(|error| error.into_inner()) = appearance;
    DIRTY.store(true, Ordering::Release);
}

/// 启动后台轮询线程，并返回第一次探测结果（超时则返回默认值）。
pub fn start() -> Appearance {
    let (first_tx, first_rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("kichi-appearance".to_owned())
        .spawn(move || watch(first_tx));

    if spawned.is_err() {
        return Appearance::default();
    }

    first_rx.recv_timeout(STARTUP_TIMEOUT).unwrap_or_default()
}

/// 主线程取走待应用的变更；没有变化时返回 `None`。
pub fn take_pending() -> Option<Appearance> {
    if !DIRTY.swap(false, Ordering::AcqRel) {
        return None;
    }
    Some(*latest().lock().unwrap_or_else(|error| error.into_inner()))
}

/// 把外观偏好交给 C++ 应用到 `QGuiApplication`。
pub fn apply(appearance: Appearance) {
    let scheme = match appearance.dark {
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    };
    let (has_accent, [r, g, b]) = match appearance.accent {
        Some(accent) => (1, accent),
        None => (0, [0.0; 3]),
    };

    // SAFETY: 只读写 QGuiApplication 的调色板，调用点都在 GUI 线程
    // （main 载入 QML 前，或 Kichi::pump 里）。
    unsafe {
        kichi_apply_appearance(scheme, has_accent, r, g, b);
    }
}

fn watch(first_tx: mpsc::Sender<Appearance>) {
    let mut settings = block_on(Settings::new()).ok();
    let mut last = read(&mut settings);
    publish(last);
    let _ = first_tx.send(last);

    loop {
        std::thread::sleep(POLL_INTERVAL);

        // 启动时门户可能还没起来；之后每轮重试一次连接。
        if settings.is_none() {
            settings = block_on(Settings::new()).ok();
        }

        let current = read(&mut settings);
        if current != last {
            last = current;
            publish(current);
        }
    }
}

fn read(settings: &mut Option<Settings<'_>>) -> Appearance {
    let mut appearance = Appearance::default();

    if let Some(settings) = settings.as_ref() {
        let (scheme, accent) = block_on(async {
            (
                settings.color_scheme().await.ok(),
                settings.accent_color().await.ok(),
            )
        });

        appearance.dark = match scheme {
            Some(ColorScheme::PreferDark) => Some(true),
            Some(ColorScheme::PreferLight) => Some(false),
            Some(ColorScheme::NoPreference) | None => None,
        };

        appearance.accent = accent.and_then(|color| {
            let rgb = [color.red(), color.green(), color.blue()];
            valid_accent(rgb).then_some(rgb)
        });
    }

    // 没有门户（Sway、旧版桌面等）时，至少别让 GTK 深色主题白设。
    if appearance.dark.is_none() {
        appearance.dark = gtk_prefers_dark();
    }

    appearance
}

fn valid_accent(rgb: [f64; 3]) -> bool {
    rgb.into_iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
}

fn gtk_prefers_dark() -> Option<bool> {
    if let Ok(theme) = std::env::var("GTK_THEME") {
        if let Some(preference) = gtk_theme_preference(&theme) {
            return Some(preference);
        }
    }

    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    for relative in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Some(preference) = read_gtk_settings(&config_dir.join(relative)) {
            return Some(preference);
        }
    }

    None
}

fn read_gtk_settings(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    gtk_settings_preference(&text)
}

/// `GTK_THEME=Adwaita:dark` / `GTK_THEME=Adwaita:light`。
fn gtk_theme_preference(theme: &str) -> Option<bool> {
    let (_, variant) = theme.rsplit_once(':')?;
    if variant.eq_ignore_ascii_case("dark") {
        Some(true)
    } else if variant.eq_ignore_ascii_case("light") {
        Some(false)
    } else {
        None
    }
}

/// 解析 `gtk-3.0/settings.ini` 或 `gtk-4.0/settings.ini` 的关键两行。
fn gtk_settings_preference(text: &str) -> Option<bool> {
    let mut prefer_dark = None;
    let mut theme_name = None;

    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');

        match key.trim() {
            "gtk-application-prefer-dark-theme" => {
                prefer_dark = Some(value == "1" || value.eq_ignore_ascii_case("true"));
            }
            "gtk-theme-name" => theme_name = Some(value.to_ascii_lowercase()),
            _ => {}
        }
    }

    if prefer_dark == Some(true) {
        return Some(true);
    }

    match theme_name.as_deref() {
        Some(name) if name.contains("dark") => Some(true),
        Some(name) if name.contains("light") => Some(false),
        _ => prefer_dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_components_must_be_in_range() {
        assert!(valid_accent([0.0, 0.5, 1.0]));
        assert!(!valid_accent([-0.01, 0.5, 0.5]));
        assert!(!valid_accent([0.5, 1.01, 0.5]));
        assert!(!valid_accent([f64::NAN, 0.5, 0.5]));
        assert!(!valid_accent([f64::INFINITY, 0.5, 0.5]));
    }

    #[test]
    fn gtk_theme_variant_is_explicit_only() {
        assert_eq!(gtk_theme_preference("Adwaita:dark"), Some(true));
        assert_eq!(gtk_theme_preference("Adwaita:light"), Some(false));
        assert_eq!(gtk_theme_preference("Adwaita"), None);
        assert_eq!(gtk_theme_preference("Breeze:Dark"), Some(true));
    }

    #[test]
    fn gtk_settings_prefer_dark_flag() {
        assert_eq!(
            gtk_settings_preference("gtk-application-prefer-dark-theme=1"),
            Some(true)
        );
        assert_eq!(
            gtk_settings_preference("gtk-application-prefer-dark-theme=0"),
            Some(false)
        );
    }

    #[test]
    fn gtk_settings_theme_name() {
        assert_eq!(
            gtk_settings_preference("gtk-theme-name=Adwaita-dark"),
            Some(true)
        );
        assert_eq!(gtk_settings_preference("gtk-theme-name=Adwaita"), None);
        assert_eq!(
            gtk_settings_preference("gtk-theme-name=Adwaita-light"),
            Some(false)
        );
    }

    #[test]
    fn gtk_settings_flag_wins_over_theme_name() {
        assert_eq!(
            gtk_settings_preference(
                "gtk-theme-name=Adwaita-light\n\
                 gtk-application-prefer-dark-theme=1"
            ),
            Some(true)
        );
    }
}
