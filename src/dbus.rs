// zbus shenanigans
#![allow(clippy::used_underscore_binding)]

use {
    std::{collections::HashMap, path::PathBuf},
    zbus::{ObjectPath, conn::Builder, message::Header, names::BusName},
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
    pub app_id: String,
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

type DBusFilt = (String, Vec<(u32, String)>);
#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FilePortal {
    #[expect(clippy::too_many_arguments)]
    async fn open_file(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, zbus::Variant<'static>>)> {
        self.fun(connection, hdr, handle, app_id, title, options, Mode::Open)
            .await
    }
    #[expect(clippy::too_many_arguments)]
    async fn save_file(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, zbus::Variant<'static>>)> {
        self.fun(connection, hdr, handle, app_id, title, options, Mode::Save)
            .await
    }
}

impl FilePortal {
    #[expect(clippy::too_many_arguments)]
    async fn fun(
        &self,
        _connection: &zbus::Connection,
        _hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
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
        self.sender
            .send(Req {
                title: title.to_owned(),
                filters,
                mode,
                dir,
                suggested_save_name,
                app_id: app_id.to_owned(),
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
