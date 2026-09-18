mod app;
mod credentials;
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kichi")
            // Wayland 下 winit 无法直接设置窗口图标; 这里声明 app_id,
            // 由 KDE/GNOME 将其与 kichi.desktop 及主题图标关联。
            .with_app_id("kichi")
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
