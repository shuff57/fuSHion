//! Left panel: picture, budget, backend, Design button, chat.

use super::worker::Backend;
use super::{ACCENT, App, ChatKind, ERROR, OK};
use egui::{Color32, RichText};

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(Color32::from_gray(140)),
    );
    ui.add_space(2.0);
}

/// Save the API key with mode 600. Never logs the key.
fn save_api_key(key: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = sf_agent::claude_api::api_key_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(key.trim().as_bytes())?;
    Ok(())
}

impl App {
    pub(super) fn design_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Design").size(16.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.settings_button(ui)
            });
        });
        ui.add_space(4.0);

        // Picture.
        section(ui, "Picture");
        let w = ui.available_width();
        let h = 150.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
        match &self.source_tex {
            Some(tex) => {
                let s = tex.size_vec2();
                let f = (rect.width() / s.x).min(rect.height() / s.y);
                let r = egui::Rect::from_center_size(rect.center(), s * f);
                painter.image(
                    tex.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop or paste a picture here\nor click Open",
                    egui::FontId::proportional(13.0),
                    Color32::from_gray(120),
                );
            }
        }
        let stroke_col = if resp.hovered() {
            ACCENT.gamma_multiply(0.6)
        } else {
            Color32::from_gray(52)
        };
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, stroke_col),
            egui::StrokeKind::Inside,
        );
        let open_clicked = resp.clicked();
        ui.add_space(4.0);
        let (open_btn, paste_btn) = ui
            .horizontal(|ui| {
                let o = ui.button("Open picture…").clicked();
                let p = ui
                    .button("Paste picture")
                    .on_hover_text("Use the image on the clipboard (Ctrl+V)")
                    .clicked();
                (o, p)
            })
            .inner;
        if paste_btn {
            self.paste_image(&ctx);
        }
        if (open_clicked || open_btn)
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                .pick_file()
        {
            self.set_image(&ctx, &p);
        }

        // Budget.
        ui.add_space(6.0);
        section(ui, "Part budget");
        let busy = self.busy.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 110.0).max(80.0);
            let r = ui.add_enabled(
                !busy,
                egui::Slider::new(&mut self.budget, 50..=2000)
                    .logarithmic(true)
                    .suffix(" parts"),
            );
            if (r.drag_stopped() || (r.changed() && !r.dragged()))
                && let Err(e) = self.session_budget_sync()
            {
                self.set_status(e, true);
            }
        });
        let r = ui
            .add_enabled(
                !busy,
                egui::Checkbox::new(&mut self.print_mode, "Build for 3D printing"),
            )
            .on_hover_text(
                "On: every brick is designed for its spot and printed, and Claude designs \
                 shaped details (petals, eyes, beaks) in SolidCraft.\n\
                 Off: real LEGO parts only, with a BrickLink parts list.",
            );
        if r.changed()
            && let Err(e) = self.session_print_sync()
        {
            self.set_status(e, true);
        }
        let r = ui
            .add_enabled(!busy && self.print_mode, egui::Checkbox::new(&mut self.curves, "Cut bricks to curves"))
            .on_hover_text(
                "Off (normal): the body is built from straight bricks that step around \
                 curves, like real LEGO. Only details (petals, leaves, eyes, beaks) get \
                 custom shapes.\n\
                 On: body bricks are also cut to the true curved surface (slower export).",
            );
        if r.changed()
            && let Err(e) = self.session_curves_sync()
        {
            self.set_status(e, true);
        }

        // Backend.
        ui.add_space(6.0);
        section(ui, "Claude");
        let api_ok = self.api_status.is_ok();
        let cc_ok = self.cc_status == Some(true);
        ui.add_enabled_ui(!busy, |ui| {
            ui.add_enabled(
                api_ok,
                egui::RadioButton::new(self.backend == Backend::Api, Backend::Api.label()),
            )
            .clicked()
            .then(|| self.backend = Backend::Api);
            if let Err(why) = &self.api_status {
                ui.label(RichText::new(why).small().weak());
            }
            ui.add_enabled(
                cc_ok,
                egui::RadioButton::new(
                    self.backend == Backend::ClaudeCode,
                    Backend::ClaudeCode.label(),
                ),
            )
            .clicked()
            .then(|| self.backend = Backend::ClaudeCode);
            match self.cc_status {
                None => {
                    ui.label(
                        RichText::new("Checking for the `claude` CLI…")
                            .small()
                            .weak(),
                    );
                }
                Some(false) => {
                    ui.label(
                        RichText::new("`claude` CLI not found on PATH")
                            .small()
                            .weak(),
                    );
                }
                Some(true) => {}
            }
        });
        let backend_ok = match self.backend {
            Backend::Api => api_ok,
            Backend::ClaudeCode => cc_ok,
        };
        let has_image = self.source_tex.is_some();
        ui.add_space(6.0);
        let design = ui.add_enabled(
            !busy && backend_ok && has_image,
            egui::Button::new(
                RichText::new("Design")
                    .strong()
                    .color(Color32::from_rgb(25, 20, 12)),
            )
            .fill(ACCENT)
            .min_size(egui::vec2(ui.available_width(), 32.0)),
        );
        let design = if !has_image {
            design.on_disabled_hover_text("Open a picture first")
        } else {
            design
        };
        if design.clicked() {
            self.start_turn(&ctx, String::new(), true, now);
        }

        // Progress.
        if let Some((b, t0)) = self.busy {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spinner();
                let secs = (now - t0).max(0.0) as u64;
                let who = match b {
                    Backend::Api => "Claude is designing",
                    Backend::ClaudeCode => "Claude Code is designing",
                };
                ui.label(RichText::new(format!("{who}… {}:{:02}", secs / 60, secs % 60)).weak());
            });
        }

        // Chat.
        ui.add_space(8.0);
        section(ui, "Conversation");
        let input_h = 64.0;
        let avail = ui.available_height() - input_h - 12.0;
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_min_height(avail.max(60.0) - 16.0);
                egui::ScrollArea::vertical()
                    .max_height(avail.max(60.0) - 16.0)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.chat.is_empty() {
                            ui.label(
                                RichText::new("Claude's notes and your edits appear here.")
                                    .weak()
                                    .small(),
                            );
                        }
                        for line in &self.chat {
                            match line.kind {
                                ChatKind::User => {
                                    ui.label(RichText::new("You").small().strong().color(ACCENT));
                                    ui.label(&line.text);
                                }
                                ChatKind::Claude => {
                                    ui.label(
                                        RichText::new("Claude")
                                            .small()
                                            .strong()
                                            .color(Color32::from_gray(170)),
                                    );
                                    ui.label(&line.text);
                                }
                                ChatKind::Progress => {
                                    ui.label(RichText::new(&line.text).small().weak());
                                }
                                ChatKind::Error => {
                                    ui.label(RichText::new(&line.text).small().color(ERROR));
                                }
                            }
                            ui.add_space(4.0);
                        }
                    });
            });
        ui.add_space(6.0);
        let can_send = !busy && backend_ok;
        ui.horizontal(|ui| {
            let te = egui::TextEdit::multiline(&mut self.input)
                .hint_text("Ask for a change, e.g. “make the ears bigger”")
                .desired_rows(2)
                .desired_width(ui.available_width() - 60.0);
            let r = ui.add_enabled(can_send, te);
            let enter = r.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            let send = ui.add_enabled(
                can_send && !self.input.trim().is_empty(),
                egui::Button::new("Send").min_size(egui::vec2(52.0, 44.0)),
            );
            if (send.clicked() || enter) && !self.input.trim().is_empty() && can_send {
                let text = self.input.trim().to_string();
                self.input.clear();
                self.start_turn(&ctx, text, false, now);
            }
        });
    }

    fn settings_button(&mut self, ui: &mut egui::Ui) {
        let resp = ui
            .add(egui::Button::new(RichText::new("Settings").small()))
            .on_hover_text("Settings");
        let force_open = self
            .debug_settings
            .then_some(egui::SetOpenCommand::Bool(true));
        egui::Popup::from_toggle_button_response(&resp)
            .open_memory(force_open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(280.0);
                ui.label(RichText::new("Anthropic API key").strong());
                ui.label(
                    RichText::new("Stored in your config folder, readable only by you.")
                        .small()
                        .weak(),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.key_input)
                            .password(true)
                            .hint_text("sk-ant-…")
                            .desired_width(190.0),
                    );
                    if ui
                        .add_enabled(!self.key_input.trim().is_empty(), egui::Button::new("Save"))
                        .clicked()
                    {
                        match save_api_key(&self.key_input) {
                            Ok(()) => {
                                self.key_saved = true;
                                self.refresh_api_status();
                                if self.api_status.is_ok() && self.busy.is_none() {
                                    self.backend = Backend::Api;
                                }
                            }
                            Err(e) => self.set_status(format!("Could not save the key: {e}"), true),
                        }
                        // Never keep the key in memory longer than needed.
                        self.key_input.clear();
                    }
                });
                if self.key_saved {
                    ui.label(RichText::new("Saved").small().color(OK));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Effort").strong());
                ui.horizontal(|ui| {
                    for e in EFFORTS {
                        ui.selectable_value(&mut self.effort, e.to_string(), e);
                    }
                });
                ui.label(
                    RichText::new("Higher effort thinks longer (API backend).")
                        .small()
                        .weak(),
                );
            });
    }
}
