//! 自定义播放器的输入占位符。
//!
//! 只有非 mpv / VLC 的通用路径使用这些占位符；mpv / VLC 的输入仍由
//! 各自的 IPC / oldrc 控制接口下发。

use std::path::Path;

use super::PlayerConfig;

impl PlayerConfig {
    /// 自定义参数是否引用了直链或临时清单。
    ///
    /// 这类命令由用户自己决定播放器从哪里拿输入，不再自动追加
    /// `--playlist=`；没有任何输入占位符时，通用路径会追加裸清单路径。
    /// mpv / VLC 的输入仍只走各自的 IPC / oldrc。
    pub(crate) fn uses_input_placeholder(&self) -> bool {
        self.uses_url_placeholder() || self.uses_playlist_placeholder()
    }

    pub(crate) fn uses_url_placeholder(&self) -> bool {
        self.pre_args
            .iter()
            .chain(self.access_args.iter())
            .any(|arg| arg.contains("{url}"))
    }

    pub(crate) fn uses_playlist_placeholder(&self) -> bool {
        self.pre_args
            .iter()
            .chain(self.access_args.iter())
            .any(|arg| arg.contains("{playlist}"))
    }
}

/// 替换自定义播放器参数里的输入占位符。
///
/// 单次扫描，替换进去的标题 / 路径 / URL 不会再次被当成占位符展开。
/// 未知占位符原样保留。
pub(crate) fn expand_play_arg(template: &str, title: &str, playlist: &Path, url: &str) -> String {
    let mut out = String::with_capacity(template.len() + title.len() + url.len());
    let mut rest = template;
    while let Some(index) = rest.find('{') {
        out.push_str(&rest[..index]);
        let after = &rest[index..];
        if let Some(tail) = after.strip_prefix("{title}") {
            out.push_str(title);
            rest = tail;
        } else if let Some(tail) = after.strip_prefix("{playlist}") {
            out.push_str(&playlist.to_string_lossy());
            rest = tail;
        } else if let Some(tail) = after.strip_prefix("{url}") {
            out.push_str(url);
            rest = tail;
        } else {
            out.push('{');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_inputs_expand_once_and_keep_unknown_placeholders() {
        let expanded = expand_play_arg(
            "--title={title} --url={url} --playlist={playlist} {unknown}",
            "{url}",
            Path::new("/tmp/kichi-play/test.m3u"),
            "https://cdn.invalid/v?sign=SECRET",
        );
        assert_eq!(
            expanded,
            "--title={url} --url=https://cdn.invalid/v?sign=SECRET \
             --playlist=/tmp/kichi-play/test.m3u {unknown}"
        );
    }

    #[test]
    fn input_placeholders_are_recognised_in_pre_and_access_args() {
        let mut config = PlayerConfig {
            program: "totem".to_owned(),
            pre_args: vec!["--title={title}".to_owned()],
            access_args: vec!["{playlist}".to_owned()],
        };
        assert!(config.uses_playlist_placeholder());
        assert!(!config.uses_url_placeholder());
        assert!(config.uses_input_placeholder());

        config.pre_args.push("{url}".to_owned());
        config.access_args.clear();
        assert!(config.uses_url_placeholder());
        assert!(!config.uses_playlist_placeholder());
        assert!(config.uses_input_placeholder());
    }
}
