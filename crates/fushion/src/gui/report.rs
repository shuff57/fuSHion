//! Right panel: selection inspector, report, export.

use super::design::section;
use super::{ACCENT, App, ERROR, OK, Tool};
use egui::{Color32, RichText};
use sf_core::{CATALOG, ColorId, Palette};

pub(super) fn swatch(ui: &mut egui::Ui, rgb: [u8; 3], size: f32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    let stroke = if selected {
        egui::Stroke::new(2.0, ACCENT)
    } else if resp.hovered() {
        egui::Stroke::new(1.0, Color32::from_gray(200))
    } else {
        egui::Stroke::new(1.0, Color32::from_gray(60))
    };
    p.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Outside);
    resp
}

/// A grid of palette swatches; returns the clicked colour.
pub(super) fn swatch_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    current: ColorId,
) -> Option<ColorId> {
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for c in &palette.colors {
            if swatch(ui, c.rgb, 18.0, c.code == current)
                .on_hover_text(&c.name)
                .clicked()
            {
                out = Some(c.code);
            }
        }
    });
    out
}

fn kv(ui: &mut egui::Ui, k: &str, v: impl Into<egui::WidgetText>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(v);
        });
    });
}

impl App {
    pub(super) fn report_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.edit_section(ui);
                self.report_section(ui);
                self.export_section(ui);
            });
    }

    fn edit_section(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.session else { return };
        let palette = s.palette.clone();
        let busy = self.busy.is_some();
        ui.label(RichText::new("Edit").size(16.0).strong());
        ui.add_space(4.0);
        ui.add_enabled_ui(!busy && s.model.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tool, Tool::Select, "Select");
                ui.selectable_value(&mut self.tool, Tool::Add, "Add part");
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    self.undo_edit();
                }
            });
        });
        if busy {
            ui.label(
                RichText::new("Editing is paused while Claude works.")
                    .small()
                    .weak(),
            );
        }
        let prev_tool = self.tool;
        match self.tool {
            Tool::Select => {
                let sel = self
                    .selected
                    .and_then(|i| self.model().and_then(|m| m.placements.get(i)).cloned());
                match sel {
                    Some(p) => {
                        let name = p
                            .def()
                            .map(|d| d.name)
                            .unwrap_or(p.part.as_str())
                            .to_string();
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            swatch(ui, palette.rgb(p.color), 14.0, false);
                            ui.label(RichText::new(name).strong());
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} · at x {}, z {}, {} plates up",
                                palette.name(p.color),
                                p.origin.x,
                                p.origin.z,
                                p.origin.y
                            ))
                            .small()
                            .weak(),
                        );
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Rotate").on_hover_text("R").clicked() {
                                    self.rotate_selected();
                                }
                                if ui
                                    .button("Delete")
                                    .on_hover_text("Delete / Backspace")
                                    .clicked()
                                {
                                    self.delete_selected();
                                }
                            });
                            ui.label(
                                RichText::new("Arrows move one stud, PageUp/PageDown one plate.")
                                    .small()
                                    .weak(),
                            );
                            ui.add_space(2.0);
                            if let Some(c) = swatch_grid(ui, &palette, p.color) {
                                self.recolor_selected(c);
                            }
                        });
                    }
                    None => {
                        ui.label(
                            RichText::new("Click a part in the viewer to select it.")
                                .small()
                                .weak(),
                        );
                    }
                }
            }
            Tool::Add => {
                ui.add_space(4.0);
                let cur = sf_core::parts::part(self.add_part)
                    .map(|d| d.name)
                    .unwrap_or(self.add_part);
                egui::ComboBox::from_id_salt("add_part")
                    .selected_text(cur)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for d in CATALOG {
                            ui.selectable_value(&mut self.add_part, d.id, d.name);
                        }
                    });
                ui.label(
                    RichText::new(
                        "Click the top of a part (or the ground) to place it. R turns it.",
                    )
                    .small()
                    .weak(),
                );
                if let Some(c) = swatch_grid(ui, &palette, self.add_color) {
                    self.add_color = c;
                }
            }
        }
        if prev_tool != self.tool {
            self.scene_dirty = true;
        }
        ui.add_space(8.0);
        ui.separator();
    }

    fn report_section(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.session else { return };
        ui.label(RichText::new("Report").size(16.0).strong());
        let Some(r) = &s.report else {
            ui.label(RichText::new("Nothing built yet. Open a picture and press Design.").weak());
            return;
        };
        let palette = &s.palette;
        ui.add_space(4.0);
        let budget = r.budget.or(s.budget).unwrap_or(self.budget);
        let frac = r.parts as f32 / budget.max(1) as f32;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}", r.parts)).size(22.0).strong());
            ui.label(RichText::new(format!("parts of {budget}")).weak());
        });
        let bar_col = if frac > 1.0 { ERROR } else { ACCENT };
        ui.add(
            egui::ProgressBar::new(frac.min(1.0))
                .desired_height(6.0)
                .fill(bar_col),
        );
        ui.add_space(4.0);
        kv(
            ui,
            "Size",
            format!(
                "{} × {} studs, {:.1} bricks tall",
                r.size_studs[0],
                r.size_studs[1],
                r.size_studs[2] as f32 / 3.0
            ),
        );
        kv(
            ui,
            "",
            RichText::new(format!(
                "{:.0} × {:.0} × {:.0} mm",
                r.size_mm[0], r.size_mm[1], r.size_mm[2]
            ))
            .weak(),
        );
        kv(ui, "Steps", format!("{}", r.steps));
        kv(ui, "Slopes", format!("{}", r.slopes));

        section(ui, "Colours");
        for c in &r.colors {
            ui.horizontal(|ui| {
                swatch(ui, palette.rgb(c.code), 14.0, false);
                ui.label(&c.color);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{}", c.parts)).weak());
                });
            });
        }

        section(ui, "Structure");
        if r.issues.is_empty() {
            ui.label(RichText::new("One connected piece, balanced, no weak joints").color(OK));
        }
        for i in &r.issues {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&i.kind).color(ERROR).strong());
                ui.label(RichText::new(&i.detail).small());
            });
            ui.label(RichText::new(&i.where_).small().weak());
        }

        if !r.custom_candidates.is_empty() {
            section(ui, "Custom part candidates");
            for c in &r.custom_candidates {
                let what = if c.color.is_empty() {
                    c.part.clone()
                } else {
                    format!("{} {}", c.part, c.color)
                };
                ui.label(RichText::new(format!("{what} × {}", c.count)).small());
                ui.label(RichText::new(&c.reason).small().weak());
            }
        }
        if !r.warnings.is_empty() {
            section(ui, "Warnings");
            for w in &r.warnings {
                ui.label(RichText::new(w).small().weak());
            }
        }
        ui.add_space(8.0);
        ui.separator();
    }

    fn export_section(&mut self, ui: &mut egui::Ui) {
        // Collect a finished export.
        if let Some(task) = &self.export_task {
            let done = task.lock().ok().and_then(|mut g| g.take());
            if let Some(result) = done {
                self.export_task = None;
                match result {
                    Ok(paths) => {
                        let n = paths.len();
                        self.exported = paths;
                        self.set_status(format!("Exported {n} files"), false);
                    }
                    Err(e) => self.set_status(format!("Export failed: {e}"), true),
                }
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        let Some(s) = &self.session else { return };
        let has_model = s.model.is_some();
        ui.label(RichText::new("Export").size(16.0).strong());
        let what = if sf_agent::custom::ENABLED {
            "LDraw model, parts list (CSV), BrickLink wanted list, and STL/3MF files for parts that need printing."
        } else {
            "LDraw model, parts list (CSV) and BrickLink wanted list."
        };
        ui.label(RichText::new(what).small().weak());
        ui.add_space(2.0);
        let busy = self.export_task.is_some();
        let label = if busy { "Exporting…" } else { "Export to folder…" };
        if ui.add_enabled(has_model && !busy, egui::Button::new(label)).clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            match s.export_job() {
                Ok(job) => {
                    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
                    let out = slot.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let r = job.run(&dir).map_err(|e| format!("{e:#}"));
                        if let Ok(mut g) = out.lock() {
                            *g = Some(r);
                        }
                        ctx.request_repaint();
                    });
                    self.export_task = Some(slot);
                }
                Err(e) => self.set_status(format!("Export failed: {e:#}"), true),
            }
        }
        for p in &self.exported {
            ui.label(RichText::new(p.display().to_string()).small().monospace().weak());
        }
    }
}
