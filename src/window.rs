use {
    crate::dbus,
    egui_file_dialog::{DialogState, FileDialog, FileFilter, Filter, SaveExtension},
    egui_sf2g::{
        SfEgui, egui,
        sf2g::{
            cpp::FBox,
            graphics::RenderWindow,
            window::{ContextSettings, Event, Style},
        },
    },
    std::time::Duration,
    zbus::zvariant::ObjectPath,
};

pub struct FileChooserWin {
    dialog: FileDialog,
    win: FBox<RenderWindow>,
    sf_egui: SfEgui,
    obj_path: ObjectPath<'static>,
    exe_path: Option<String>,
}

fn apply_froggy_style(ctx: &egui::Context) {
    const fn c(r: u8, g: u8, b: u8) -> egui::Color32 {
        egui::Color32::from_rgb(r, g, b)
    }
    ctx.global_style_mut(|style| {
        let vis = &mut style.visuals;
        vis.panel_fill = c(123, 170, 0);
        vis.window_fill = c(123, 170, 0);
        vis.window_stroke.color = c(180, 233, 0);
        // Things like...
        // Text edit bg
        vis.extreme_bg_color = c(45, 97, 21);
        // Check box bg
        vis.widgets.inactive.bg_fill = c(145, 147, 0);
        // Button fill
        vis.widgets.inactive.weak_bg_fill = c(154, 193, 0);
        // Button text
        vis.widgets.inactive.fg_stroke.color = c(206, 255, 103);
        // Hovered selectable label bg
        vis.widgets.hovered.weak_bg_fill = c(73, 126, 0);
        // Hovered selectable label stroke
        vis.widgets.hovered.bg_stroke.color = c(187, 242, 58);
        // Separator
        vis.widgets.noninteractive.bg_stroke.color = c(173, 214, 0);
        // Noninteractive label text
        vis.widgets.noninteractive.fg_stroke.color = c(220, 255, 0);
        // Clicked selecable label bg
        vis.widgets.active.weak_bg_fill = c(74, 178, 136);
        // Selected label
        vis.selection.bg_fill = c(186, 216, 0);
        vis.selection.stroke.color = c(255, 255, 255);
    });
}

// Naive conversion from a glob pattern to a save extension
fn conv_patterns_to_save_ext(pats: &[glob::Pattern]) -> String {
    let mut out = String::new();
    if let Some(pat) = pats.first() {
        let s = pat.as_str();
        for &b in s.as_bytes() {
            if b.is_ascii_lowercase() {
                out.push(b as char);
            }
        }
    }
    if out.is_empty() {
        out = "unknown".into();
    }
    out
}

pub fn spawn_window(
    mut req: dbus::Req,
    windows: &mut Vec<FileChooserWin>,
    frog_cfg: &crate::Config,
) {
    let mut win = RenderWindow::new(
        (640, 412),
        &req.title,
        Style::TITLEBAR,
        &ContextSettings::default(),
    )
    .unwrap();
    win.set_vertical_sync_enabled(true);
    let mut dialog = FileDialog::new();
    if let Some(exe_path) = &req.exe_path
        && let Some(storage) = frog_cfg.per_app_file_dia_storage.get(exe_path)
    {
        *dialog.storage_mut() = storage.clone();
    }
    let cfg = dialog.config_mut();
    cfg.title_bar = false;
    cfg.fixed_pos = Some(egui::pos2(0., 0.));
    cfg.resizable = false;
    cfg.as_modal = false;
    for filt in req.filters.drain(..) {
        match req.mode {
            dbus::Mode::Open => {
                let filter = Filter::new(move |path: &std::path::Path| {
                    if path.is_dir() {
                        return true;
                    }
                    for pat in &filt.patterns {
                        if pat.matches_path(path) {
                            return true;
                        }
                    }
                    false
                });
                let file_filt = FileFilter {
                    id: egui::Id::new(&filt.name),
                    name: filt.name.clone(),
                    filter,
                };
                cfg.file_filters.push(file_filt);
                cfg.default_file_filter = Some(filt.name);
            }
            dbus::Mode::Save => {
                let save_ext = SaveExtension {
                    id: egui::Id::new(&filt.name),
                    name: filt.name.clone(),
                    file_extension: conv_patterns_to_save_ext(&filt.patterns),
                };
                cfg.save_extensions.push(save_ext);
                cfg.default_save_extension = Some(filt.name);
            }
        }
    }
    match req.mode {
        dbus::Mode::Open => {
            dialog.pick_file();
        }
        dbus::Mode::Save => {
            if !req.suggested_save_name.is_empty() {
                cfg.default_file_name = req.suggested_save_name;
            }
            dialog.save_file();
        }
    }

    let sf_egui = SfEgui::new(&win);
    apply_froggy_style(sf_egui.context());
    windows.push(FileChooserWin {
        dialog,
        win,
        sf_egui,
        obj_path: req.obj_path,
        exe_path: req.exe_path,
    });
}

pub fn update_windows(
    windows: &mut Vec<FileChooserWin>,
    conn: &zbus::blocking::Connection,
    cfg: &mut crate::Config,
) {
    let mut any = false;
    windows.retain_mut(|win| {
        let mut retain = true;
        any = true;
        while let Some(ev) = win.win.poll_event() {
            if ev == Event::Closed {
                retain = false;
            }
            win.sf_egui.add_event(&ev);
        }
        let di = win
            .sf_egui
            .run(&mut win.win, |_rw, ui| {
                win.dialog.update(ui);
                if *win.dialog.state() == DialogState::Cancelled {
                    dbus::emit_response(conn, win.obj_path.clone(), dbus::RePayload::UserCancel)
                        .unwrap();
                    retain = false;
                }
                if let Some(picked) = win.dialog.take_picked() {
                    dbus::emit_response(
                        conn,
                        win.obj_path.clone(),
                        dbus::RePayload::PickedFiles(vec![picked]),
                    )
                    .unwrap();
                    retain = false;
                }
            })
            .unwrap();
        win.sf_egui.draw(di, &mut win.win, None);
        win.win.display();
        if !retain {
            if let Some(exe_path) = &win.exe_path {
                cfg.per_app_file_dia_storage
                    .insert(exe_path.clone(), win.dialog.storage_mut().clone());
            }
            if let Err(e) = cfg.save() {
                eprintln!("Failed to save config: {e}");
            }
            conn.object_server()
                .remove::<dbus::RequestPortalFacade, _>(&win.obj_path)
                .unwrap();
        }
        retain
    });
    if !any {
        std::thread::sleep(Duration::from_millis(250));
    }
}
