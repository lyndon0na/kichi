//! 统一主题与视觉参数: 根据深/浅色输出配色、圆角与间距。
//! 不依赖具体业务, 仅做全局样式的入口。

use eframe::egui::{Color32, Context, CornerRadius, Margin, Stroke, Vec2, Visuals};

/// 混合: base 上叠 fg, 透明度 alpha(0..1)。
pub fn mix(base: Color32, fg: Color32, alpha: f32) -> Color32 {
    let t = alpha.clamp(0.0, 1.0);
    let b = base.to_array();
    let f = fg.to_array();
    Color32::from_rgb(
        (b[0] as f32 * (1.0 - t) + f[0] as f32 * t).round() as u8,
        (b[1] as f32 * (1.0 - t) + f[1] as f32 * t).round() as u8,
        (b[2] as f32 * (1.0 - t) + f[2] as f32 * t).round() as u8,
    )
}

pub struct Theme {
    pub dark: bool,
    /// 品牌强调色。
    pub accent: Color32,
    /// 强调色上的文字(白)。
    pub on_accent: Color32,
    /// 主内容区背景。
    pub bg: Color32,
    /// 侧边栏 / 面板背景(比主背景稍深或稍浅)。
    pub panel: Color32,
    /// 卡片背景。
    pub card: Color32,
    /// 行悬浮背景。
    pub hover: Color32,
    /// 边框 / 分隔线。
    pub border: Color32,
    /// 主文字。
    pub text: Color32,
    /// 次级文字。
    pub text_weak: Color32,
    /// 最弱文字 / 占位。
    pub text_faint: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        if dark {
            Theme {
                dark,
                accent: Color32::from_rgb(108, 132, 255),
                on_accent: Color32::from_rgb(255, 255, 255),
                bg: Color32::from_rgb(19, 20, 24),
                panel: Color32::from_rgb(26, 28, 34),
                card: Color32::from_rgb(30, 32, 39),
                hover: Color32::from_rgba_unmultiplied(255, 255, 255, 14),
                border: Color32::from_rgb(46, 48, 58),
                text: Color32::from_rgb(236, 238, 244),
                text_weak: Color32::from_rgb(154, 159, 172),
                text_faint: Color32::from_rgb(104, 108, 120),
                ok: Color32::from_rgb(92, 186, 128),
                warn: Color32::from_rgb(220, 168, 78),
                danger: Color32::from_rgb(235, 98, 86),
            }
        } else {
            Theme {
                dark,
                accent: Color32::from_rgb(74, 96, 230),
                on_accent: Color32::from_rgb(255, 255, 255),
                bg: Color32::from_rgb(245, 246, 249),
                panel: Color32::from_rgb(255, 255, 255),
                card: Color32::from_rgb(255, 255, 255),
                hover: Color32::from_rgba_unmultiplied(15, 20, 45, 14),
                border: Color32::from_rgb(226, 229, 236),
                text: Color32::from_rgb(28, 30, 35),
                text_weak: Color32::from_rgb(96, 102, 114),
                text_faint: Color32::from_rgb(150, 156, 168),
                ok: Color32::from_rgb(46, 160, 102),
                warn: Color32::from_rgb(190, 138, 40),
                danger: Color32::from_rgb(214, 66, 54),
            }
        }
    }

    /// 强调色的弱化打底(供选中/标签等使用)。
    pub fn accent_soft(&self) -> Color32 {
        mix(self.bg, self.accent, if self.dark { 0.16 } else { 0.12 })
    }
}

/// 将配色写入全局 egui 样式(每次主题变化后调用)。
pub fn configure(ctx: &Context, theme: &Theme) {
    let mut v = if theme.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.override_text_color = Some(theme.text);
    v.panel_fill = theme.panel;
    v.window_fill = theme.card;
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, theme.border);
    v.faint_bg_color = mix(theme.bg, theme.text, if theme.dark { 0.035 } else { 0.03 });
    v.code_bg_color = mix(theme.bg, theme.text, if theme.dark { 0.08 } else { 0.05 });
    v.hyperlink_color = theme.accent;
    v.warn_fg_color = theme.warn;
    v.error_fg_color = theme.danger;
    v.selection.bg_fill = theme.accent;
    v.selection.stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    v.clip_rect_margin = 8.0;

    let radius = CornerRadius::same(9);
    v.widgets.noninteractive.corner_radius = radius;
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.corner_radius = radius;
    v.widgets.open.corner_radius = radius;

    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme.text_weak);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, theme.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, theme.text);
    v.widgets.active.fg_stroke = Stroke::new(1.0, theme.text);

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme.border);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, theme.border);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.accent);
    v.widgets.active.bg_stroke = Stroke::new(1.0, theme.accent);

    v.widgets.noninteractive.weak_bg_fill = mix(theme.card, theme.text, if theme.dark { 0.06 } else { 0.04 });
    v.widgets.inactive.weak_bg_fill = mix(theme.bg, theme.text, if theme.dark { 0.09 } else { 0.055 });
    v.widgets.hovered.weak_bg_fill = mix(theme.bg, theme.accent, if theme.dark { 0.18 } else { 0.14 });
    v.widgets.active.weak_bg_fill = mix(theme.bg, theme.accent, if theme.dark { 0.26 } else { 0.2 });
    v.widgets.inactive.bg_fill = mix(theme.bg, theme.text, if theme.dark { 0.06 } else { 0.04 });
    v.widgets.hovered.bg_fill = mix(theme.bg, theme.text, if theme.dark { 0.12 } else { 0.06 });
    v.widgets.active.bg_fill = mix(theme.bg, theme.text, if theme.dark { 0.16 } else { 0.09 });

    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.window_margin = Margin::same(14);
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.indent = 14.0;
    ctx.set_style(style);
}
