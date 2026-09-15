//! 轻量矢量图标库(纯 painter 绘制, 不依赖字体字形)。
//! 所有图标在 0..16 的规范坐标里定义, 按目标矩形等比缩放描画。

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// 文件夹
    Folder,
    /// 视频(矩形屏 + 播放三角)
    Video,
    /// 图片(相框 + 山/日)
    Image,
    /// 音频(八分音符)
    Audio,
    /// 文档(页 + 横线)
    Doc,
    /// 压缩包(盒 + 拉链)
    Archive,
    /// 通用文件(页 + 折角)
    File,
    /// 下载(箭头落盘)
    Download,
    /// 上级 / 返回(朝上箭头)
    Up,
    /// 设置(齿轮)
    Gear,
    /// 传输任务(上下箭头)
    Transfer,
    /// 刷新(循环箭头)
    Refresh,
    /// 退出(门)
    Logout,
    /// 搜索(放大镜)
    Search,
    /// 关闭 / 清除(叉)
    Close,
    /// 分享(节点连线)
    Share,
    /// 回收站(垃圾桶)
    Trash,
}

/// 为方便描线, 生成画布坐标闭包。
struct Canvas {
    x0: f32,
    y0: f32,
    s: f32,
    color: Color32,
}

impl Canvas {
    fn new(rect: Rect, color: Color32) -> Self {
    let s = rect.width().min(rect.height());
    let x0 = rect.center().x - s / 2.0;
    let y0 = rect.center().y - s / 2.0;
    Canvas { x0, y0, s, color }
    }
    /// 规范坐标 -> 屏幕坐标。
    fn p(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.x0 + (x / 16.0) * self.s, self.y0 + (y / 16.0) * self.s)
    }
    fn stroke(&self, width: f32) -> Stroke {
        Stroke::new(width.max(1.0), self.color)
    }
    /// 依次连接多段折线。
    fn polyline(&self, painter: &Painter, pts: &[[f32; 2]], width: f32) {
        for w in pts.windows(2) {
            painter.line_segment(
                [self.p(w[0][0], w[0][1]), self.p(w[1][0], w[1][1])],
                self.stroke(width),
            );
        }
    }
    /// 描边闭合多边形(带填充可选)。
    fn polygon(&self, painter: &Painter, pts: &[[f32; 2]], fill: Option<Color32>, width: f32) {
        let v: Vec<Pos2> = pts.iter().map(|&[x, y]| self.p(x, y)).collect();
        if let Some(f) = fill {
            painter.add(Shape::convex_polygon(v, f, Stroke::NONE));
        } else {
            painter.add(Shape::closed_line(v, self.stroke(width)));
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn rect_filled(&self, painter: &Painter, x0: f32, y0: f32, x1: f32, y1: f32, radius: f32, c: Color32) {
        let r = Rect::from_min_max(self.p(x0, y0), self.p(x1, y1));
        painter.rect_filled(r, radius, c);
    }
    #[allow(clippy::too_many_arguments)]
    fn rect_stroke(&self, painter: &Painter, x0: f32, y0: f32, x1: f32, y1: f32, radius: f32, width: f32) {
        let r = Rect::from_min_max(self.p(x0, y0), self.p(x1, y1));
        painter.rect_stroke(r, radius, self.stroke(width), eframe::egui::StrokeKind::Inside);
    }
}

/// 在 `rect` 内绘制 `glyph`, 使用 `color` 作为前景色。
pub fn paint(painter: &Painter, rect: Rect, glyph: Glyph, color: Color32) {
    let c = Canvas::new(rect, color);
    match glyph {
        Glyph::Folder => folder(painter, &c),
        Glyph::Video => video(painter, &c),
        Glyph::Image => image(painter, &c),
        Glyph::Audio => audio(painter, &c),
        Glyph::Doc => doc(painter, &c, true),
        Glyph::File => doc(painter, &c, false),
        Glyph::Archive => archive(painter, &c),
        Glyph::Download => download(painter, &c),
        Glyph::Up => chevron(painter, &c),
        Glyph::Gear => gear(painter, &c),
        Glyph::Transfer => transfer(painter, &c),
        Glyph::Refresh => refresh(painter, &c),
        Glyph::Logout => logout(painter, &c),
        Glyph::Search => search(painter, &c),
        Glyph::Close => close(painter, &c),
        Glyph::Share => share(painter, &c),
        Glyph::Trash => trash(painter, &c),
    }
}

fn chevron(painter: &Painter, c: &Canvas) {
    // 朝上箭头(上级)
    c.polyline(painter, &[[4.0, 9.6], [8.0, 5.6], [12.0, 9.6]], 1.8);
}

fn mix_white(color: Color32, t: f32) -> Color32 {
    let c = color.to_array();
    Color32::from_rgb(
        (c[0] as f32 + (255.0 - c[0] as f32) * t).round() as u8,
        (c[1] as f32 + (255.0 - c[1] as f32) * t).round() as u8,
        (c[2] as f32 + (255.0 - c[2] as f32) * t).round() as u8,
    )
}

fn folder(painter: &Painter, c: &Canvas) {
    // 开口文件夹: 背面(浅) + 前面挡板(主色)
    let back = mix_white(c.color, 0.34);
    // 背面露出上沿
    c.rect_filled(painter, 2.0, 3.8, 14.0, 13.8, 1.8, back);
    // 前面挡板(压住下 2/3)
    c.rect_filled(painter, 2.0, 7.0, 14.0, 13.8, 1.6, c.color);
    // 背面右侧留一条主色小边, 增强“开口”层次
    painter.rect_filled(
        Rect::from_min_max(c.p(2.0, 3.8), c.p(3.2, 13.8)),
        1.0,
        mix_white(c.color, 0.22),
    );
}

fn video(painter: &Painter, c: &Canvas) {
    c.rect_stroke(painter, 2.2, 3.0, 13.8, 13.0, 2.4, 1.5);
    c.polygon(
        painter,
        &[[5.6, 5.6], [5.6, 10.4], [11.0, 8.0]],
        Some(c.color),
        1.0,
    );
}

fn image(painter: &Painter, c: &Canvas) {
    c.rect_stroke(painter, 2.2, 2.6, 13.8, 13.4, 2.4, 1.5);
    painter.circle_filled(c.p(11.2, 5.6), c.s * 0.09, c.color);
    c.polyline(painter, &[[3.4, 11.4], [6.6, 7.6], [9.4, 10.4], [12.6, 6.8]], 1.4);
}

fn audio(painter: &Painter, c: &Canvas) {
    // 两条符干 + 符梁 + 两个符头
    c.rect_filled(painter, 4.1, 3.8, 6.0, 10.8, 1.0, c.color);
    c.rect_filled(painter, 9.6, 2.8, 11.5, 9.4, 1.0, c.color);
    c.rect_filled(painter, 4.6, 3.4, 11.1, 4.5, 0.5, c.color);
    painter.circle_filled(c.p(5.0, 11.8), c.s * 0.105, c.color);
    painter.circle_filled(c.p(10.5, 10.2), c.s * 0.105, c.color);
}

fn doc(painter: &Painter, c: &Canvas, with_lines: bool) {
    c.rect_stroke(painter, 2.6, 2.0, 13.4, 14.0, 1.8, 1.4);
    // 右上折角
    c.polyline(painter, &[[9.4, 2.0], [13.4, 6.0]], 1.4);
    c.polyline(painter, &[[9.4, 6.0], [9.4, 2.0]], 1.4);
    if with_lines {
        c.polyline(painter, &[[4.4, 8.6], [11.6, 8.6]], 1.4);
        c.polyline(painter, &[[4.4, 11.0], [9.6, 11.0]], 1.4);
    }
}

fn archive(painter: &Painter, c: &Canvas) {
    // 盒体
    c.rect_stroke(painter, 2.2, 6.4, 13.8, 13.4, 1.6, 1.5);
    // 盒盖
    c.polyline(painter, &[[2.2, 6.4], [5.0, 3.2], [11.0, 3.2], [13.8, 6.4]], 1.5);
    c.polyline(painter, &[[5.0, 3.2], [5.0, 5.0]], 1.5);
    c.polyline(painter, &[[11.0, 3.2], [11.0, 5.0]], 1.5);
    // 拉链
    c.polyline(painter, &[[8.0, 6.4], [8.0, 13.4]], 1.3);
    c.polyline(painter, &[[6.4, 7.4], [7.0, 6.8]], 1.2);
    c.polyline(painter, &[[6.8, 9.0], [7.4, 8.4]], 1.2);
    c.polyline(painter, &[[6.4, 10.6], [7.0, 10.0]], 1.2);
    c.polyline(painter, &[[6.8, 12.2], [7.4, 11.6]], 1.2);
    c.polyline(painter, &[[9.6, 7.4], [9.0, 6.8]], 1.2);
    c.polyline(painter, &[[9.2, 9.0], [8.6, 8.4]], 1.2);
    c.polyline(painter, &[[9.6, 10.6], [9.0, 10.0]], 1.2);
    c.polyline(painter, &[[9.2, 12.2], [8.6, 11.6]], 1.2);
}

fn download(painter: &Painter, c: &Canvas) {
    c.polyline(painter, &[[8.0, 2.6], [8.0, 10.2]], 1.7);
    c.polyline(painter, &[[4.4, 6.9], [8.0, 11.2], [11.6, 6.9]], 1.7);
    c.polyline(painter, &[[3.4, 13.4], [12.6, 13.4]], 1.7);
}

fn gear(painter: &Painter, c: &Canvas) {
    let n = 8;
    let inner = c.s * 0.26;
    let outer = c.s * 0.34;
    let center = c.p(8.0, 8.0);
    let tooth_h = (c.s * 0.5).max(1.2);
    for i in 0..n {
        let a = std::f32::consts::TAU * (i as f32) / (n as f32) + std::f32::consts::TAU / (2.0 * n as f32);
        let dir = eframe::egui::vec2(a.cos(), a.sin());
        let p0 = center + dir * inner;
        let p1 = center + dir * outer;
        let r = center + dir * (c.s * 0.05);
        let perpendicular = eframe::egui::vec2(-dir.y, dir.x);
        let p0a = r + perpendicular * tooth_h * 0.16;
        let p0b = r + perpendicular * -tooth_h * 0.16;
        painter.line_segment([p0, p1], c.stroke(1.2));
        painter.line_segment([p0a, p0b], c.stroke(1.6));
    }
    painter.circle(center, c.s * 0.17, Color32::TRANSPARENT, c.stroke(1.5));
    painter.circle_filled(center, c.s * 0.045, c.color);
}

fn transfer(painter: &Painter, c: &Canvas) {
    // 上传/下载: 左侧实心下箭头 + 右侧实心上箭头
    c.polyline(painter, &[[5.5, 2.6], [5.5, 11.2]], 1.6);
    c.polygon(
        painter,
        &[[3.9, 10.9], [7.1, 10.9], [5.5, 13.6]],
        Some(c.color),
        1.0,
    );
    c.polyline(painter, &[[10.5, 13.4], [10.5, 4.8]], 1.6);
    c.polygon(
        painter,
        &[[12.1, 5.1], [8.9, 5.1], [10.5, 2.4]],
        Some(c.color),
        1.0,
    );
}

fn refresh(painter: &Painter, c: &Canvas) {
    // 顺时针圆弧(右上留缺口) + 末端实心箭头
    let center = c.p(8.0, 8.0);
    let r = c.s * 0.30;
    // 屏幕坐标: 0rad = 向右, 顺时针为正(y 向下)。
    let start = -0.5f32;
    let end = std::f32::consts::PI * 1.5; // 正上方
    let sweep = end - start;
    let n = 48;
    let mut pts: Vec<Pos2> = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let a = start + sweep * (i as f32) / (n as f32);
        pts.push(center + egui::vec2(a.cos(), a.sin()) * r);
    }
    painter.add(Shape::line(pts, c.stroke(1.8)));

    // 末端箭头: 沿圆弧切线方向(顺时针)的实心三角
    let dir = egui::vec2(end.cos(), end.sin());
    let tan = egui::vec2(-end.sin(), end.cos());
    let base = center + dir * r;
    let tip = base + tan * (c.s * 0.17);
    let w = dir * (c.s * 0.09);
    painter.add(Shape::convex_polygon(
        vec![tip, base + w, base - w],
        c.color,
        Stroke::NONE,
    ));
}

fn logout(painter: &Painter, c: &Canvas) {
    c.polyline(painter, &[[5.0, 2.6], [2.6, 2.6], [2.6, 13.4], [5.0, 13.4]], 1.4);
    c.polyline(painter, &[[8.0, 5.0], [11.0, 8.0], [8.0, 11.0]], 1.5);
    c.polyline(painter, &[[5.0, 8.0], [11.0, 8.0]], 1.5);
}

fn search(painter: &Painter, c: &Canvas) {
    // 镜片 + 手柄
    painter.circle(c.p(7.0, 7.0), c.s * 0.30, Color32::TRANSPARENT, c.stroke(1.5));
    painter.line_segment([c.p(9.4, 9.4), c.p(13.2, 13.2)], c.stroke(1.6));
}

fn close(painter: &Painter, c: &Canvas) {
    painter.line_segment([c.p(4.6, 4.6), c.p(11.4, 11.4)], c.stroke(1.6));
    painter.line_segment([c.p(11.4, 4.6), c.p(4.6, 11.4)], c.stroke(1.6));
}

fn share(painter: &Painter, c: &Canvas) {
    // 右侧两个节点 + 左侧一个节点, 三线相连。
    let a = c.p(11.8, 4.2);
    let b = c.p(4.2, 8.0);
    let d = c.p(11.8, 11.8);
    painter.line_segment([b, a], c.stroke(1.4));
    painter.line_segment([b, d], c.stroke(1.4));
    painter.circle_filled(a, c.s * 0.11, c.color);
    painter.circle_filled(b, c.s * 0.11, c.color);
    painter.circle_filled(d, c.s * 0.11, c.color);
}

fn trash(painter: &Painter, c: &Canvas) {
    // 桶盖 + 提手
    c.polyline(painter, &[[3.2, 4.4], [12.8, 4.4]], 1.5);
    c.polyline(painter, &[[6.4, 4.4], [6.9, 2.6], [9.1, 2.6], [9.6, 4.4]], 1.4);
    // 桶身
    c.polyline(painter, &[[4.6, 4.4], [5.3, 13.6]], 1.4);
    c.polyline(painter, &[[11.4, 4.4], [10.7, 13.6]], 1.4);
    c.polyline(painter, &[[5.3, 13.6], [10.7, 13.6]], 1.4);
    // 内部竖纹
    c.polyline(painter, &[[6.9, 6.6], [7.1, 11.4]], 1.2);
    c.polyline(painter, &[[9.1, 6.6], [8.9, 11.4]], 1.2);
}
