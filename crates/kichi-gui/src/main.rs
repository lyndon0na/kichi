mod app;
mod cache;
mod credentials;
mod filetypes;
mod format;
mod icons;
mod kde;
mod logging;
mod msg;
mod settings;
mod theme;
mod worker;

use eframe::egui;

fn main() -> eframe::Result {
    logging::init();
    // Wayland 下窗口图标由 app_id 关联桌面文件与图标主题: Flatpak 里应用 ID
    // 就是沙箱标识(io.github.lyndon0na.Kichi), 原生运行时用 desktop 文件同名。
    let app_id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "kichi".to_string());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kichi")
            .with_app_id(app_id)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 560.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Kichi",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
