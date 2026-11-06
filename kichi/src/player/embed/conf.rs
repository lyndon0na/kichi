//! kichi 自己的 mpv 配置：`embed-mpv.conf`（mpv.conf 的顶层选项子集）。
//!
//! 只解析顶层选项，`[profile]` 段整段跳过；`include` 递归读取，循环与
//! 过深 include 只记警告。内嵌必需的选项在 [`RESERVED`] 里，用户文件里
//! 写了也不生效——它们由 `create_mpv` 在解析结果之后统一覆盖。
//!
//! 解析失败不阻断播放：单行坏了只跳过该行，文件读不了只记警告。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 内嵌必需或被安全边界限制的选项：用户配置里出现时忽略。
const RESERVED: &[&str] = &[
    // 内嵌渲染链路固定项（ADR 0027）
    "vo",
    "gpu-context",
    "gpu-api",
    "clipboard-backends",
    "config",
    "config-dir",
    "idle",
    "force-window",
    "terminal",
    "msg-level",
    // 输入 / 键位 / 脚本：kichi 接管键盘，不加载用户 input.conf 与脚本
    "input-default-bindings",
    "input-vo-keyboard",
    "input-conf",
    "input-ipc-server",
    "load-scripts",
    "script",
    "script-opts",
    // 界面 / 生命周期由 kichi 控制
    "osc",
    "osd-level",
    "ytdl",
    "audio-display",
    "sub-auto",
    "fullscreen",
    "fs",
    "pause",
    "start",
];

/// include 的最大递归层级；超过只警告，不读取。
const MAX_INCLUDE_DEPTH: usize = 8;

/// 首次打开「编辑」时生成的模板；全部是注释，不会改变播放行为。
pub(crate) const TEMPLATE: &str = r#"# kichi 内嵌播放器的 mpv 配置（mpv.conf 语法）
# 设置 → 播放 → 内嵌播放器 →「embed-mpv.conf」打开后生效，改动在下次开播时读取。
#
# 内嵌必需的选项由 kichi 固定，这里写了也不会生效：
#   vo / gpu-context / gpu-api / clipboard-backends / input-* / osc / config /
#   idle / force-window / ytdl / sub-auto / script* / fullscreen 等。
# 当前内嵌固定 gpu-api=opengl，Vulkan 相关的 gpu-api / gpu-context 不生效。
#
# 可以复用已有的 mpv 配置；只读取顶层选项，profile 段与脚本不会加载：
# include=~/.config/mpv/mpv.conf
#
# 常用示例（取消注释后生效）：
# profile=gpu-hq
# hwdec=vaapi
# scale=ewa_lanczossharp
# deband=yes
# tone-mapping=bt.2446a
# glsl-shaders=~/.config/mpv/shaders/Anime4K_Clamp_Highlights.glsl:~/.config/mpv/shaders/Anime4K_Upscale_CNN_x2_M.glsl
"#;

/// 一次解析的结果。
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// 按文件顺序排列的顶层选项；重复项保留，后出现的覆盖前面的。
    pub options: Vec<(String, String)>,
    /// 读文件 / include 的问题；只用于日志，不阻断播放。
    pub warnings: Vec<String>,
    /// 因为是内嵌必需项而忽略的行数。
    pub ignored: usize,
}

impl Parsed {
    /// 解析主文件及其 include；文件不存在时返回空结果。
    pub fn load(path: &Path) -> Self {
        let mut parsed = Self::default();
        let mut visited = HashSet::new();
        read_file(path, 0, &mut visited, &mut parsed);
        parsed
    }
}

fn read_file(path: &Path, depth: usize, visited: &mut HashSet<PathBuf>, out: &mut Parsed) {
    if depth > MAX_INCLUDE_DEPTH {
        out.warnings.push(format!(
            "include 层级超过 {MAX_INCLUDE_DEPTH}，已跳过 {}",
            path.display()
        ));
        return;
    }
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(key) {
        out.warnings
            .push(format!("忽略循环 include：{}", path.display()));
        return;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            out.warnings
                .push(format!("读取 {} 失败：{err}", path.display()));
            return;
        }
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut in_profile = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        // 顶层选项才是无条件生效的；profile 段里的条件选项不在这里展开。
        if line.starts_with('[') && line.ends_with(']') {
            in_profile = true;
            continue;
        }
        if in_profile {
            continue;
        }
        let line = line.strip_prefix("--").unwrap_or(line);
        let (name, value) = match line.split_once('=') {
            Some((name, value)) => (name.trim(), value.trim()),
            // 裸选项：`deband` 是开，`no-deband` 是关。
            None => match line.strip_prefix("no-") {
                Some(name) => (name.trim(), "no"),
                None => (line, "yes"),
            },
        };
        if name.is_empty() {
            out.warnings.push(format!(
                "{}:{} 选项名为空，已跳过",
                path.display(),
                index + 1
            ));
            continue;
        }
        if name.eq_ignore_ascii_case("include") {
            match include_path(value, base) {
                Some(include) => read_file(&include, depth + 1, visited, out),
                None => out.warnings.push(format!(
                    "{}:{} include 路径为空，已跳过",
                    path.display(),
                    index + 1
                )),
            }
            continue;
        }
        if RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(name))
        {
            out.ignored += 1;
            continue;
        }
        out.options.push((name.to_owned(), value.to_owned()));
    }
}

fn include_path(value: &str, base: &Path) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if value.is_empty() {
        return None;
    }
    if value == "~" {
        return std::env::var_os("HOME").map(PathBuf::from);
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join(rest));
        }
    }
    let path = PathBuf::from(value);
    Some(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

/// 文件不存在时写一份全注释模板；已存在则原样保留。
pub(crate) fn ensure_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(err) => return Err(format!("创建 embed-mpv.conf 失败：{err}")),
    };
    use std::io::Write;
    file.write_all(TEMPLATE.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|err| format!("写入 embed-mpv.conf 失败：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kichi-embed-conf-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建测试目录");
        dir
    }

    fn parse(name: &str, body: &str) -> (Parsed, PathBuf) {
        let dir = test_dir(name);
        let path = dir.join("embed-mpv.conf");
        std::fs::write(&path, body).expect("写配置");
        (Parsed::load(&path), dir)
    }

    #[test]
    fn parses_top_level_options_comments_and_flags() {
        let (parsed, dir) = parse(
            "basic",
            "\u{feff}# 注释\n\n; 另一种注释\nscale=ewa_lanczossharp\n--deband=yes\ndeband\nno-hwdec\n",
        );
        assert_eq!(
            parsed.options,
            vec![
                ("scale".to_owned(), "ewa_lanczossharp".to_owned()),
                ("deband".to_owned(), "yes".to_owned()),
                ("deband".to_owned(), "yes".to_owned()),
                ("hwdec".to_owned(), "no".to_owned()),
            ]
        );
        assert!(parsed.warnings.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reserved_options_and_profile_sections_are_skipped() {
        let (parsed, dir) = parse(
            "reserved",
            "vo=vulkan\ngpu-api=vulkan\ninput-conf=~/x\nfullscreen=yes\n[my-profile]\nscale=bilinear\n",
        );
        assert!(parsed.options.is_empty(), "profile 段与保留项都不生效");
        assert_eq!(parsed.ignored, 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn includes_are_relative_and_cycle_safe() {
        let dir = test_dir("include");
        let main = dir.join("embed-mpv.conf");
        let extra = dir.join("extra.conf");
        std::fs::write(&main, "include=extra.conf\ninclude=extra.conf\n").expect("写主配置");
        std::fs::write(&extra, "include=embed-mpv.conf\nscale=ewa_lanczos\n").expect("写子配置");
        let parsed = Parsed::load(&main);
        assert_eq!(
            parsed.options,
            vec![("scale".to_owned(), "ewa_lanczos".to_owned())]
        );
        assert!(
            parsed.warnings.iter().any(|line| line.contains("循环")),
            "重复 include 要被识别为循环：{:?}",
            parsed.warnings
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_is_an_empty_result() {
        let dir = test_dir("missing");
        let parsed = Parsed::load(&dir.join("nope.conf"));
        assert!(parsed.options.is_empty());
        assert!(parsed.warnings.is_empty(), "文件不存在是正常状态");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_file_writes_template_without_overwriting() {
        let dir = test_dir("template");
        let path = dir.join("nested").join("embed-mpv.conf");
        ensure_file(&path).expect("生成模板");
        let body = std::fs::read_to_string(&path).expect("读模板");
        assert!(body.contains("glsl-shaders"), "模板要给出可直接改的示例");
        assert!(
            body.lines().all(|line| line.trim_start().starts_with('#')),
            "模板必须全部注释，避免默认改变播放行为"
        );

        std::fs::write(&path, "scale=bilinear\n").expect("用户改写");
        ensure_file(&path).expect("已存在时不应覆盖");
        assert_eq!(
            std::fs::read_to_string(&path).expect("读回"),
            "scale=bilinear\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
