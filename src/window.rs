use {
    crate::dbus,
    egui_file_dialog::{DialogState, FileDialog},
    egui_sf2g::{
        SfEgui, egui,
        sf2g::{
            cpp::FBox,
            graphics::RenderWindow,
            window::{ContextSettings, Event, Style},
        },
    },
    std::time::Duration,
};

pub struct FileChooserWin {
    dialog: FileDialog,
    win: FBox<RenderWindow>,
    sf_egui: SfEgui,
    req: dbus::Req,
}

fn apply_froggy_style(ctx: &egui::Context) {
    const fn c(r: u8, g: u8, b: u8) -> egui::Color32 {
        egui::Color32::from_rgb(r, g, b)
    }
    ctx.style_mut(|style| {
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

pub fn spawn_window(req: dbus::Req, windows: &mut Vec<FileChooserWin>, frog_cfg: &crate::Config) {
    let mut win = RenderWindow::new(
        (640, 412),
        &req.title,
        Style::TITLEBAR,
        &ContextSettings::default(),
    )
    .unwrap();
    win.set_vertical_sync_enabled(true);
    let mut dialog = FileDialog::new();
    *dialog.storage_mut() = frog_cfg.file_dia_storage.clone();
    let cfg = dialog.config_mut();
    cfg.title_bar = false;
    cfg.fixed_pos = Some(egui::pos2(0., 0.));
    cfg.resizable = false;
    cfg.as_modal = false;
    dialog.pick_file();
    let sf_egui = SfEgui::new(&win);
    apply_froggy_style(sf_egui.context());
    windows.push(FileChooserWin {
        dialog,
        win,
        sf_egui,
        req,
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
            .run(&mut win.win, |_rw, ctx| {
                win.dialog.update(ctx);
                if win.dialog.state() == DialogState::Cancelled {
                    dbus::emit_response(conn, win.req.path.clone(), dbus::RePayload::UserCancel)
                        .unwrap();
                    retain = false;
                }
                if let Some(picked) = win.dialog.take_picked() {
                    dbus::emit_response(
                        conn,
                        win.req.path.clone(),
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
            cfg.file_dia_storage = win.dialog.storage_mut().clone();
            if let Err(e) = cfg.save() {
                eprintln!("Failed to save config: {e}");
            }
            conn.object_server()
                .remove::<dbus::RequestPortalFacade, _>(&win.req.path)
                .unwrap();
        }
        retain
    });
    if !any {
        std::thread::sleep(Duration::from_millis(250));
    }
}
