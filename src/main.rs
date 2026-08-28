mod theme;
mod zoom;

use eframe::egui;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

const TOP_DIR: &str = "/SNS/VENUS";

/// Recursively set rwx-for-all on a path and everything inside it.
/// Entries owned by other users cannot be chmod'ed and are left as-is.
fn open_permissions_recursive(path: &Path) {
    let open_perms = fs::Permissions::from_mode(0o777);
    let _ = fs::set_permissions(path, open_perms);
    if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                open_permissions_recursive(&entry.path());
            }
        }
    }
}

/// Check if the current user has read+execute access to a directory
/// using the POSIX access() syscall, which respects groups and ACLs.
fn has_read_access(path: &Path) -> bool {
    if let Some(path_str) = path.to_str() {
        if let Ok(c_path) = CString::new(path_str) {
            return unsafe { libc::access(c_path.as_ptr(), libc::R_OK | libc::X_OK) } == 0;
        }
    }
    false
}

fn main() -> eframe::Result {
    let mut options = eframe::NativeOptions::default();
    options.viewport = options.viewport.with_inner_size(egui::vec2(500.0, 800.0));
    eframe::run_native(
        "Marimo Application Launcher",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(theme::load());
            cc.egui_ctx.set_zoom_factor(zoom::load());
            // Type ramp and spacing are the same in both modes.
            cc.egui_ctx.all_styles_mut(|style| {
                style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
                style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(16.0));
                style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
                style.spacing.item_spacing = egui::vec2(8.0, 12.0);
                style.spacing.button_padding = egui::vec2(16.0, 8.0);
            });
            // In dark mode this portal paints on pure black, not egui's
            // default dark gray.
            let mut dark = egui::Visuals::dark();
            dark.panel_fill = egui::Color32::BLACK;
            dark.window_fill = egui::Color32::BLACK;
            cc.egui_ctx.set_visuals_of(egui::Theme::Dark, dark);
            Ok(Box::new(MyApp::new()))
        }),
    )
}

/// Fill and border of the list / description boxes: near-black surfaces on the
/// black dark background, white on the light gray light background.
fn container_colors(visuals: &egui::Visuals) -> (egui::Color32, egui::Color32) {
    if visuals.dark_mode {
        (
            egui::Color32::from_rgb(25, 25, 30),
            egui::Color32::from_rgb(80, 80, 100),
        )
    } else {
        (
            egui::Color32::WHITE,
            egui::Color32::from_rgb(190, 190, 205),
        )
    }
}

struct MyApp {
    folders: Vec<String>,
    selected: Option<usize>,
    filter_text: String,
    py_files: Vec<String>,
    selected_py: Option<usize>,
    description: Option<String>,
    launch_time: Option<Instant>,
    launch_error: Option<String>,
}

impl MyApp {
    fn new() -> Self {
        let mut folders = Vec::new();
        if let Ok(entries) = fs::read_dir(Path::new(TOP_DIR)) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.starts_with("IPTS-") && has_read_access(&path) {
                            folders.push(name.to_string());
                        }
                    }
                }
            }
        }
        folders.sort();
        Self {
            folders,
            selected: None,
            filter_text: String::new(),
            py_files: Vec::new(),
            selected_py: None,
            description: None,
            launch_time: None,
            launch_error: None,
        }
    }

    fn refresh_py_files(&mut self) {
        self.py_files.clear();
        self.selected_py = None;
        if let Some(i) = self.selected {
            let notebooks_dir = Path::new(TOP_DIR)
                .join(&self.folders[i])
                .join("shared")
                .join("notebooks");
            if let Ok(entries) = fs::read_dir(&notebooks_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with("_marimo.py") {
                                self.py_files.push(name.to_string());
                            }
                        }
                    }
                }
            }
            self.py_files.sort();
        }
    }

    fn refresh_description(&mut self) {
        self.description = None;
        if let (Some(folder_idx), Some(py_idx)) = (self.selected, self.selected_py) {
            let file_path = Path::new(TOP_DIR)
                .join(&self.folders[folder_idx])
                .join("shared")
                .join("notebooks")
                .join(&self.py_files[py_idx]);
            if let Ok(content) = fs::read_to_string(&file_path) {
                for line in content.lines().take(20) {
                    let trimmed = line.trim();
                    if trimmed.starts_with("description") {
                        if let Some(value) = trimmed.split('=').nth(1) {
                            let value = value.trim().trim_matches('"').trim_matches('\'');
                            if !value.is_empty() {
                                self.description = Some(value.to_string());
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Theme and style are installed once at startup (see `main`).

        if self.selected_py.is_some() {
            egui::TopBottomPanel::bottom("bottom_panel")
                .frame(
                    egui::Frame::new()
                        .fill(ctx.style().visuals.panel_fill)
                        .inner_margin(12.0),
                )
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        if let Some(err) = &self.launch_error {
                            ui.colored_label(ui.visuals().error_fg_color, err);
                        }
                        let launching = self.launch_time
                            .map(|t| t.elapsed().as_secs() < 5)
                            .unwrap_or(false);

                        if launching {
                            ui.add_enabled(false, egui::Button::new("\u{231b} Launching..."));
                            ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        } else {
                            if ui.button("Launch this application").clicked() {
                                if let (Some(folder_idx), Some(py_idx)) = (self.selected, self.selected_py) {
                                    let py_file = Path::new(TOP_DIR)
                                        .join(&self.folders[folder_idx])
                                        .join("shared")
                                        .join("notebooks")
                                        .join(&self.py_files[py_idx]);
                                    // Make sure the notebooks folder and everything inside it
                                    // is read/write/exec by everybody, whether it was just
                                    // created or already there.
                                    if let Some(notebooks_dir) = py_file.parent() {
                                        open_permissions_recursive(notebooks_dir);
                                    }
                                    let marimo_bin = "/SNS/VENUS/shared/software/git/marimo_notebooks/.pixi/envs/default/bin/marimo";
                                    println!("Launching: {} run {}", marimo_bin, py_file.display());
                                    match Command::new(marimo_bin)
                                        .arg("run")
                                        .arg(&py_file)
                                        // IPTS notebook folders have no
                                        // pyproject.toml/.marimo.toml, so
                                        // marimo would use its 8 MB default
                                        // and truncate large cell outputs.
                                        .env("MARIMO_OUTPUT_MAX_BYTES", "20000000")
                                        .spawn()
                                    {
                                        Ok(_) => {
                                            self.launch_error = None;
                                            self.launch_time = Some(Instant::now());
                                        }
                                        Err(e) => {
                                            self.launch_error = Some(format!(
                                                "Failed to launch {}: {}",
                                                marimo_bin, e
                                            ));
                                            self.launch_time = None;
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("marimo_dedicated_main_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        theme::toggle_button(ui);
                        zoom::toggle_button(ui);
                    });
                    ui.add_space(10.0);

                    let label_width = 80.0;
                    let field_width = ui.available_width() - label_width - 20.0;

                    // IPTS search filter
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(label_width, ui.spacing().interact_size.y),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| { ui.strong("Search"); },
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter_text)
                                .desired_width(field_width)
                                .hint_text("Type to filter IPTS folders..."),
                        );
                    });

                    // Always-visible IPTS list
                    let prev_selected = self.selected;
                    let filtered: Vec<(usize, &String)> = self.folders.iter().enumerate()
                        .filter(|(_, name)| {
                            self.filter_text.is_empty()
                                || name.to_lowercase().contains(&self.filter_text.to_lowercase())
                        })
                        .collect();

                    let (container_fill, container_border) = container_colors(ui.visuals());
                    egui::Frame::new()
                        .fill(container_fill)
                        .stroke(egui::Stroke::new(1.0, container_border))
                        .corner_radius(4.0)
                        .inner_margin(4.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .min_scrolled_height(350.0)
                                .max_height(350.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    for (i, folder) in &filtered {
                                        if ui.selectable_label(
                                            self.selected == Some(*i),
                                            *folder,
                                        ).clicked() {
                                            self.selected = Some(*i);
                                        }
                                    }
                                });
                        });

                    if self.selected != prev_selected {
                        self.refresh_py_files();
                        self.description = None;
                    }

                    // Notebook selector or "no application" message
                    if self.selected.is_some() && self.py_files.is_empty() {
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.add_space(label_width + 8.0);
                            ui.colored_label(ui.visuals().warn_fg_color, "No application found");
                        });
                    } else if !self.py_files.is_empty() {
                        let prev_selected_py = self.selected_py;
                        let current_py_label = match self.selected_py {
                            Some(i) => self.py_files[i].as_str(),
                            None => "Select a notebook...",
                        };
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(label_width, ui.spacing().interact_size.y),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| { ui.strong("Notebook"); },
                            );
                            egui::ComboBox::from_id_salt("py_combo")
                                .selected_text(current_py_label)
                                .width(field_width)
                                .show_ui(ui, |ui| {
                                    for (i, file) in self.py_files.iter().enumerate() {
                                        ui.selectable_value(&mut self.selected_py, Some(i), file);
                                    }
                                });
                        });

                        if self.selected_py != prev_selected_py {
                            self.refresh_description();
                        }

                        if self.selected_py.is_some() {
                            // Description box
                            if let Some(desc) = &self.description {
                                ui.add_space(5.0);
                                let (container_fill, container_border) = container_colors(ui.visuals());
                                egui::Frame::new()
                                    .fill(container_fill)
                                    .stroke(egui::Stroke::new(1.0, container_border))
                                    .corner_radius(6.0)
                                    .inner_margin(12.0)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(desc);
                                    });
                            }
                        }
                    }
                });
        });
    }
}
