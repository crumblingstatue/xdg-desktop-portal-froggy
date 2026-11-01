// zbus shenanigans
#![allow(clippy::used_underscore_binding)]

use std::fmt::Write as _;

use {
    std::{collections::HashMap, path::PathBuf},
    zbus::{ObjectPath, conn::Builder},
};

pub struct FilePortal {
    sender: std::sync::mpsc::Sender<Req>,
}

pub fn make_file_portal() -> (FilePortal, std::sync::mpsc::Receiver<Req>) {
    let (send, recv) = std::sync::mpsc::channel();
    (FilePortal { sender: send }, recv)
}

#[derive(Debug)]
pub enum Mode {
    Open,
    Save,
}

#[derive(Debug)]
pub struct Req {
    pub mode: Mode,
    /// Whether the thing we want to open is a directory
    pub dir: bool,
    pub title: String,
    pub filters: Vec<Filter>,
    pub suggested_save_name: String,
    pub app_unique_id: Option<String>,
    pub sender: tokio::sync::mpsc::Sender<(u32, HashMap<String, zbus::Variant<'static>>)>,
}

#[derive(Debug)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<glob::Pattern>,
}

impl Filter {
    fn from_dbus(filt: DBusFilt) -> Option<Self> {
        let mut yes = true;
        let this = Self {
            name: filt.0,
            patterns: filt
                .1
                .into_iter()
                .map(|(kind, pat)| {
                    assert_eq!(kind, 0);
                    // We already have "All Files" by default
                    if pat == "*" {
                        yes = false;
                    }
                    glob::Pattern::new(&pat).unwrap()
                })
                .collect(),
        };
        yes.then_some(this)
    }
}

use x11rb::{properties::WmClass, protocol::xproto::Window};

fn get_wm_class(window: Window) -> Result<String, Box<dyn std::error::Error>> {
    let (conn, _) = x11rb::connect(None)?;

    WmClass::get(&conn, window)?.reply()?.map_or_else(
        || Err("Failed to get WM class".into()),
        |wm_class| Ok(String::from_utf8_lossy(wm_class.class()).into_owned()),
    )
}

type DBusFilt = (String, Vec<(u32, String)>);
#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FilePortal {
    async fn open_file(
        &self,
        _handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, zbus::Variant<'static>>)> {
        self.fun(app_id, parent_window, title, options, Mode::Open)
            .await
    }
    async fn save_file(
        &self,
        _handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, zbus::Variant<'static>>)> {
        self.fun(app_id, parent_window, title, options, Mode::Save)
            .await
    }
}

impl FilePortal {
    async fn fun(
        &self,
        xdg_app_id: &str,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::Variant<'_>>,
        mode: Mode,
    ) -> zbus::fdo::Result<(u32, HashMap<String, zbus::Variant<'static>>)> {
        let filters = options.get("filters").map_or_else(Vec::new, |val| {
            let filters: Vec<DBusFilt> = val.clone().try_into().unwrap();
            filters.into_iter().filter_map(Filter::from_dbus).collect()
        });
        let suggested_save_name = options.get("current_name").map_or_else(String::new, |val| {
            let name: String = val.clone().try_into().unwrap();
            name
        });
        let dir = options
            .get("directory")
            .is_some_and(|val| val.try_into().unwrap());
        let (send, mut recv) = tokio::sync::mpsc::channel(1);
        let mut app_unique_id: Option<String> = None;
        if !xdg_app_id.is_empty() {
            write!(app_unique_id.get_or_insert_default(), "id:{xdg_app_id};").unwrap();
        }
        if let Some(mut x11_win_id) = parent_window.strip_prefix("x11:") {
            // Strip 0x prefix if necessary
            if let Some(prefix) = x11_win_id.strip_prefix("0x") {
                x11_win_id = prefix;
            }
            let id = u32::from_str_radix(x11_win_id.trim(), 16).unwrap();
            let wm_class = get_wm_class(id).unwrap();
            write!(app_unique_id.get_or_insert_default(), "class:{wm_class};").unwrap();
        }
        self.sender
            .send(Req {
                title: title.to_owned(),
                filters,
                mode,
                dir,
                suggested_save_name,
                app_unique_id,
                sender: send,
            })
            .unwrap();
        Ok(recv.recv().await.unwrap())
    }
}

pub async fn make_connection(portal: FilePortal) -> zbus::Result<zbus::Connection> {
    Builder::session()
        .name("org.freedesktop.impl.portal.desktop.froggy")
        .serve_at("/org/freedesktop/portal/desktop", portal)
        .build()
        .await
}

pub enum RePayload {
    PickedFiles(Vec<PathBuf>),
    UserCancel,
}

impl RePayload {
    pub fn into_zvariant(
        self,
    ) -> zbus::DynamicTuple<(u32, HashMap<String, zbus::Variant<'static>>)> {
        match self {
            Self::PickedFiles(path_bufs) => zbus::DynamicTuple((
                0u32,
                HashMap::<String, zbus::Variant>::from_iter([(
                    "uris".into(),
                    path_bufs
                        .into_iter()
                        .map(|buf| format!("file:///{}", buf.display()))
                        .collect::<Vec<_>>()
                        .into(),
                )]),
            )),
            Self::UserCancel => {
                zbus::DynamicTuple((1u32, HashMap::<String, zbus::Variant>::default()))
            }
        }
    }
}
