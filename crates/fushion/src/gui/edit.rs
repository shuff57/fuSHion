//! Centre area (viewer, toolbar, build steps), keyboard shortcuts and manual
//! edits. Every edit goes through `commit`, which pushes an undo snapshot and
//! calls `Session::set_model` (saves, recomputes steps and report).

use super::viewer::{self, Preset};
use super::{ACCENT, App, Tool};
use egui::{Color32, Key, RichText};
use sf_core::{Cell, ColorId, Model, Placement, Rot};

const UNDO_LIMIT: usize = 100;

/// A deferred keyboard action (input is read under a lock first).
type Action = Box<dyn FnOnce(&mut App)>;

impl App {
    fn commit(&mut self, f: impl FnOnce(&mut Model) -> bool) {
        if self.busy.is_some() {
            return;
        }
        let Some(s) = &mut self.session else { return };
        let Some(model) = s.model.clone() else { return };
        let mut next = model.clone();
        if !f(&mut next) {
            return;
        }
        match s.set_model(next) {
            Ok(_) => {
                self.undo.push(model);
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
                self.model_mtime = super::mtime(&s.dir.join("model.json"));
                self.clamp_state();
            }
            Err(e) => self.set_status(format!("Saving the edit failed: {e:#}"), true),
        }
    }

    pub(super) fn undo_edit(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let Some(prev) = self.undo.pop() else { return };
        let Some(s) = &mut self.session else { return };
        match s.set_model(prev) {
            Ok(_) => {
                self.model_mtime = super::mtime(&s.dir.join("model.json"));
                self.clamp_state();
                self.set_status("Undone", false);
            }
            Err(e) => self.set_status(format!("Undo failed: {e:#}"), true),
        }
    }

    pub(super) fn delete_selected(&mut self) {
        let Some(i) = self.selected else { return };
        self.commit(|m| {
            if i < m.placements.len() {
                m.placements.remove(i);
                true
            } else {
                false
            }
        });
        self.selected = None;
        self.scene_dirty = true;
    }

    pub(super) fn recolor_selected(&mut self, c: ColorId) {
        let Some(i) = self.selected else { return };
        self.commit(|m| match m.placements.get_mut(i) {
            Some(p) if p.color != c => {
                p.color = c;
                true
            }
            _ => false,
        });
    }

    pub(super) fn rotate_selected(&mut self) {
        let Some(i) = self.selected else { return };
        self.commit(|m| match m.placements.get_mut(i) {
            Some(p) => {
                p.rot = p.rot.then(Rot(1));
                true
            }
            None => false,
        });
    }

    fn nudge_selected(&mut self, dx: i32, dy: i32, dz: i32) {
        let Some(i) = self.selected else { return };
        self.commit(|m| match m.placements.get_mut(i) {
            Some(p) if p.origin.y + dy >= 0 => {
                p.origin = Cell::new(p.origin.x + dx, p.origin.y + dy, p.origin.z + dz);
                true
            }
            _ => false,
        });
    }

    fn add_part_at(&mut self, p: Placement) {
        let mut new_index = None;
        self.commit(|m| {
            m.placements.push(p);
            new_index = Some(m.placements.len() - 1);
            true
        });
        // Keep adding: no selection change, but remember what was placed.
        if let Some(i) = new_index {
            self.set_status(format!("Added part #{i}"), false);
        }
    }

    /// Grid axis (dx, dz) most aligned with a horizontal world direction.
    fn axis_step(v: [f32; 3]) -> (i32, i32) {
        if v[0].abs() >= v[2].abs() {
            (v[0].signum() as i32, 0)
        } else {
            (0, v[2].signum() as i32)
        }
    }

    pub(super) fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (r, _, d) = self.cam.basis();
        let fwd = Self::axis_step([d[0], 0.0, d[2]]);
        let right = Self::axis_step([r[0], 0.0, r[2]]);
        let mut actions: Vec<Action> = Vec::new();
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::COMMAND, Key::Z) {
                actions.push(Box::new(|a| a.undo_edit()));
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                actions.push(Box::new(|a| {
                    a.selected = None;
                    a.tool = Tool::Select;
                    a.scene_dirty = true;
                }));
            }
            if i.consume_key(egui::Modifiers::NONE, Key::R) {
                actions.push(Box::new(|a| {
                    if a.tool == Tool::Add {
                        a.add_rot = a.add_rot.then(Rot(1));
                        a.ghost = None;
                        a.scene_dirty = true;
                    } else {
                        a.rotate_selected();
                    }
                }));
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, Key::Backspace)
            {
                actions.push(Box::new(|a| a.delete_selected()));
            }
            let moves = [
                (Key::ArrowUp, fwd.0, 0, fwd.1),
                (Key::ArrowDown, -fwd.0, 0, -fwd.1),
                (Key::ArrowRight, right.0, 0, right.1),
                (Key::ArrowLeft, -right.0, 0, -right.1),
                (Key::PageUp, 0, 1, 0),
                (Key::PageDown, 0, -1, 0),
            ];
            for (k, dx, dy, dz) in moves {
                if i.consume_key(egui::Modifiers::NONE, k) {
                    actions.push(Box::new(move |a| a.nudge_selected(dx, dy, dz)));
                }
            }
        });
        for a in actions {
            a(self);
        }
    }

    /// The part the Add tool would place for a hover ray.
    fn ghost_for(&self, o: [f32; 3], dir: [f32; 3]) -> Option<Placement> {
        let model = self.model()?;
        let (col, y) = match viewer::pick(model, &self.vis.visible, o, dir) {
            Some((i, hit, n)) if n[1] > 0.5 => {
                let p = &model.placements[i];
                ((hit[0].floor() as i32, hit[2].floor() as i32), p.top())
            }
            Some(_) => return None,
            None => {
                // The ground plane.
                if dir[1] >= -1e-4 {
                    return None;
                }
                let t = -o[1] / dir[1];
                let hit = viewer::add(o, viewer::mul(dir, t));
                ((hit[0].floor() as i32, hit[2].floor() as i32), 0)
            }
        };
        let mut p = Placement::new(
            self.add_part,
            self.add_color,
            Cell::new(col.0, y, col.1),
            self.add_rot,
        );
        // Centre the footprint on the clicked column.
        let [fx, fz] = p.footprint();
        p.origin.x -= (fx - 1) / 2;
        p.origin.z -= (fz - 1) / 2;
        Some(p)
    }

    pub(super) fn centre(&mut self, ui: &mut egui::Ui, now: f64) {
        let full = ui.max_rect();
        let bar_h = 44.0;
        let viewer_rect =
            egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.max.y - bar_h));
        let bar_rect =
            egui::Rect::from_min_max(egui::pos2(full.min.x, full.max.y - bar_h), full.max);
        self.viewer_aspect = viewer::aspect_of(viewer_rect);

        // Step playback.
        if self.playing {
            let steps = self.model().map(|m| m.steps.len()).unwrap_or(0);
            if now - self.last_tick > 0.7 {
                self.last_tick = now;
                if self.step + 1 < steps {
                    self.step += 1;
                    self.scene_dirty = true;
                } else {
                    self.playing = false;
                }
            }
            ui.ctx().request_repaint();
        }
        if self.fit_pending && viewer_rect.width() > 10.0 {
            self.fit();
            self.fit_pending = false;
        }
        if self.scene_dirty {
            self.rebuild_scene();
        }
        if self.selected.is_some() {
            // Gentle pulse on the selection tint.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Screenshot mode can simulate one click once the camera has settled.
        let fake = if self.shot.as_ref().is_some_and(|s| s.frames == 4) {
            self.debug_click
        } else {
            None
        };
        let resp = viewer::show(
            ui,
            viewer_rect,
            &mut self.cam,
            &self.scene,
            ACCENT,
            now,
            fake,
        );

        // Add-mode ghost follows the pointer.
        if self.tool == Tool::Add && self.busy.is_none() {
            let g = resp.hover.and_then(|(o, d)| self.ghost_for(o, d));
            if g != self.ghost {
                self.ghost = g;
                self.scene_dirty = true;
            }
        } else if self.ghost.is_some() {
            self.ghost = None;
            self.scene_dirty = true;
        }
        if let Some((o, d)) = resp.clicked {
            match self.tool {
                Tool::Select => {
                    let hit = self
                        .model()
                        .and_then(|m| viewer::pick(m, &self.vis.visible, o, d))
                        .map(|h| h.0);
                    if hit != self.selected {
                        self.selected = hit;
                        self.scene_dirty = true;
                    }
                }
                Tool::Add => {
                    if let Some(p) = self.ghost_for(o, d) {
                        self.add_part_at(p);
                    }
                }
            }
        }

        // Toolbar overlay.
        let tb = egui::Rect::from_min_size(
            viewer_rect.min + egui::vec2(10.0, 10.0),
            egui::vec2(viewer_rect.width() - 20.0, 30.0),
        );
        let mut tbui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(tb)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(24, 26, 30, 225))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(&mut tbui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for p in Preset::ALL {
                        if ui
                            .add(egui::Button::new(RichText::new(p.label()).small()).frame(false))
                            .clicked()
                        {
                            self.cam.set_preset(p);
                            self.fit();
                        }
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new(RichText::new("Fit").small()).frame(false))
                        .on_hover_text("Frame the model")
                        .clicked()
                    {
                        self.fit();
                    }
                    let edges_label = RichText::new("Edges").small();
                    if ui
                        .add(egui::Button::selectable(self.edges, edges_label).frame(false))
                        .clicked()
                    {
                        self.edges = !self.edges;
                        self.scene_dirty = true;
                    }
                });
            });
        if self.model().is_none() {
            ui.painter().text(
                viewer_rect.center(),
                egui::Align2::CENTER_CENTER,
                "No model yet",
                egui::FontId::proportional(16.0),
                Color32::from_gray(110),
            );
        }
        if self.tool == Tool::Add {
            ui.painter().text(
                viewer_rect.left_bottom() + egui::vec2(12.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                "Add part · click to place · R turns · Esc stops",
                egui::FontId::proportional(12.0),
                ACCENT,
            );
        }

        self.steps_bar(ui, bar_rect);
    }

    fn steps_bar(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            egui::Stroke::new(1.0, Color32::from_gray(44)),
        );
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(egui::vec2(12.0, 6.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut bar;
        let (steps, parts) = self
            .model()
            .map(|m| (m.steps.len(), m.placements.len()))
            .unwrap_or((0, 0));
        if steps == 0 {
            ui.label(RichText::new("Build steps appear once there is a model.").weak());
            return;
        }
        if ui
            .add(egui::Button::selectable(self.step_mode, "Build steps"))
            .clicked()
        {
            self.step_mode = !self.step_mode;
            self.playing = false;
            self.scene_dirty = true;
        }
        if !self.step_mode {
            ui.label(RichText::new(format!("Showing all {parts} parts · {steps} steps")).weak());
            return;
        }
        let mut step = self.step;
        let play_label = if self.playing { "Pause" } else { "Play" };
        if ui
            .button(play_label)
            .on_hover_text("Play / pause")
            .clicked()
        {
            if !self.playing && step + 1 >= steps {
                step = 0;
            }
            self.playing = !self.playing;
            self.last_tick = ui.input(|i| i.time);
        }
        if ui.button("First").clicked() {
            step = 0;
        }
        if ui.button("‹").on_hover_text("Previous step").clicked() {
            step = step.saturating_sub(1);
        }
        if ui.button("›").on_hover_text("Next step").clicked() {
            step = (step + 1).min(steps - 1);
        }
        if ui.button("Last").clicked() {
            step = steps - 1;
        }
        let added = self
            .model()
            .and_then(|m| m.steps.get(step))
            .map(|s| s.len())
            .unwrap_or(0);
        let info = format!("Step {} of {} · +{} parts", step + 1, steps, added);
        let info_w = 170.0;
        ui.spacing_mut().slider_width = (ui.available_width() - info_w - 16.0).max(60.0);
        let mut k = step + 1;
        if ui
            .add(egui::Slider::new(&mut k, 1..=steps).show_value(false))
            .changed()
        {
            step = k - 1;
            self.playing = false;
        }
        ui.label(RichText::new(info).monospace().small());
        if step != self.step {
            self.step = step;
            self.scene_dirty = true;
        }
    }
}
