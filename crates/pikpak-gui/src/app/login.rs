use eframe::egui::{self, FontId, Frame, Key, Margin, Pos2, RichText, Stroke, vec2};

use crate::msg::Cmd;
use crate::theme::Theme;

use super::helpers::input;
use super::App;

impl App {
    pub(super) fn login_ui(&mut self, ctx: &egui::Context, th: &Theme) {
        const FW: f32 = 320.0;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(th.bg))
            .show(ctx, |ui| {
                let h = ui.available_height();
                let pad = ((h - 470.0) / 2.0).max(26.0);
                ui.add_space(pad);
                ui.vertical_centered(|ui| {
                    // Logo
                    let (r, _) = ui.allocate_exact_size(vec2(62.0, 62.0), egui::Sense::hover());
                    let lp = ui.painter().clone();
                    let rr = r;
                    lp.rect_filled(rr, th.cr(16), th.accent);
                    let g = lp.layout_no_wrap("P".into(), FontId::proportional(32.0), th.on_accent);
                    lp.galley(
                        Pos2::new(rr.center().x - g.size().x / 2.0, rr.center().y - g.size().y / 2.0 - 1.0),
                        g,
                        th.on_accent,
                    );

                    ui.add_space(16.0);
                    ui.label(RichText::new("登录 PikPak").size(22.0).strong().color(th.text));
                    ui.add_space(2.0);
                    ui.label(RichText::new("继续使用你的云端文件").size(12.5).color(th.text_weak));

                    ui.add_space(18.0);
                    if self.auth_checking {
                        ui.spinner();
                        ui.add_space(6.0);
                        ui.label(RichText::new("正在恢复登录状态 / 登录中…").color(th.text_weak));
                        return;
                    }

                    // 表单卡片
                    egui::Frame::new()
                        .fill(th.card)
                        .stroke(Stroke::new(1.0, th.border))
                        .corner_radius(th.cr(14))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            ui.set_width(FW);
                            ui.label(RichText::new("账号").size(12.0).color(th.text_weak));
                            ui.add_space(4.0);
                            ui.add(
                                input(&mut self.login_username)
                                    .desired_width(FW)
                                    .hint_text("邮箱 / 手机号"),
                            );
                            ui.add_space(12.0);
                            ui.label(RichText::new("密码").size(12.0).color(th.text_weak));
                            ui.add_space(4.0);
                            ui.add(
                                input(&mut self.login_password)
                                    .desired_width(FW)
                                    .password(true)
                                    .hint_text("密码"),
                            );

                            ui.add_space(10.0);
                            let remember = ui.checkbox(
                                &mut self.remember_password,
                                RichText::new("记住密码").size(13.0).color(th.text_weak),
                            );
                            if remember.changed() {
                                self.persist_settings();
                            }

                            ui.add_space(16.0);
                            let ok = !self.login_username.trim().is_empty()
                                && !self.login_password.is_empty();
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            let btn = egui::Button::new(
                                RichText::new("登 录").color(th.on_accent).size(15.0),
                            )
                            .fill(th.accent)
                            .stroke(Stroke::NONE)
                            .corner_radius(th.cr(10))
                            .min_size(vec2(FW, 38.0));
                            let btn_resp = ui.add_enabled(ok || enter, btn);
                            if (btn_resp.clicked() || (enter && ok)) && !self.auth_checking {
                                let username = self.login_username.trim().to_string();
                                let password = std::mem::take(&mut self.login_password);
                                if self.remember_password {
                                    self.pending_remember = Some(password.clone());
                                }
                                self.auth_error = None;
                                self.auth_checking = true;
                                self.send(Cmd::Login { username, password });
                            }
                        });

                    if let Some(err) = &self.auth_error {
                        ui.add_space(12.0);
                        ui.label(RichText::new(err).color(th.danger).size(13.0));
                    }

                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("登录遇到验证码时请稍后再试或检查网络")
                            .color(th.text_faint)
                            .size(11.5),
                    );
                });
            });
    }
}
