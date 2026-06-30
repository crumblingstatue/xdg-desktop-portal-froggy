// zbus shenanigans
#![allow(clippy::used_underscore_binding)]

use {
    std::{collections::HashMap, path::PathBuf},
    zbus::{
        ObjectServer,
        blocking::connection::Builder,
        message::Header,
        names::BusName,
        object_server::SignalEmitter,
        zvariant::{self, ObjectPath},
    },
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
    pub obj_path: ObjectPath<'static>,
    pub filters: Vec<Filter>,
    pub suggested_save_name: String,
    /// Path of executable making the request. Can be used to uniquely identify the application
    pub exe_path: Option<String>,
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

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FilePortal {
    async fn open_file(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
        _parent_window: &str,
        title: &str,
        options: HashMap<&str, zvariant::Value<'_>>,
    ) -> Result<ObjectPath<'_>, zbus::fdo::Error> {
        self.fun(connection, hdr, server, title, options, Mode::Open)
            .await
    }
    async fn save_file(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
        _parent_window: &str,
        title: &str,
        options: HashMap<&str, zvariant::Value<'_>>,
    ) -> Result<ObjectPath<'_>, zbus::fdo::Error> {
        self.fun(connection, hdr, server, title, options, Mode::Save)
            .await
    }
}

async fn get_pid(connection: &zbus::Connection, sender: &str) -> zbus::Result<u32> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;

    let pid: u32 = proxy.call("GetConnectionUnixProcessID", &(sender)).await?;

    Ok(pid)
}

fn app_path_from_pid(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

impl FilePortal {
    async fn fun(
        &self,
        connection: &zbus::Connection,
        hdr: Header<'_>,
        server: &ObjectServer,
        title: &str,
        options: HashMap<&str, zvariant::Value<'_>>,
        mode: Mode,
    ) -> Result<ObjectPath<'_>, zbus::fdo::Error> {
        let sender = hdr.sender().unwrap();
        let mut sender = sender.to_string();
        let exe_path: Option<String> = {
            let pid = get_pid(connection, &sender).await;
            match pid {
                Ok(pid) => app_path_from_pid(pid),
                Err(e) => {
                    eprintln!("{e}");
                    None
                }
            }
        };

        sender = sender.strip_prefix(":").unwrap().replace('.', "_");

        let token: &str = match options.get("handle_token") {
            Some(zvariant::Value::Str(val)) => val,
            _ => {
                return Err(zbus::fdo::Error::InvalidArgs(
                    "Missing handle_token(str)".into(),
                ));
            }
        };
        let filters = options.get("filters").map_or_else(Vec::new, |val| {
            let filters: Vec<DBusFilt> = val.clone().try_into().unwrap();
            filters.into_iter().filter_map(Filter::from_dbus).collect()
        });
        let path = ObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .unwrap();
        server.at(&path, RequestPortalFacade).await.unwrap();
        let suggested_save_name = options.get("current_name").map_or_else(String::new, |val| {
            let name: String = val.clone().try_into().unwrap();
            name
        });
        let dir = options
            .get("directory")
            .is_some_and(|val| val.try_into().unwrap());
        self.sender
            .send(Req {
                title: title.to_owned(),
                obj_path: path.clone(),
                filters,
                mode,
                dir,
                suggested_save_name,
                exe_path,
            })
            .unwrap();
        Ok(path)
    }
}

pub struct RequestPortalFacade;

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl RequestPortalFacade {
    #[allow(clippy::unused_self)]
    const fn close(&self) {}
    #[zbus(signal)]
    async fn response(
        emitter: SignalEmitter<'_>,
        response: u32,
        results: Vec<HashMap<String, zvariant::Value<'_>>>,
    ) -> zbus::Result<()>;
}

pub fn make_connection(portal: FilePortal) -> zbus::Result<zbus::blocking::Connection> {
    Builder::session()
        .unwrap()
        .name("org.freedesktop.portal.Desktop")?
        .serve_at("/org/freedesktop/portal/desktop", portal)?
        .build()
}

pub fn emit_response(
    conn: &zbus::blocking::Connection,
    path: ObjectPath<'static>,
    payload: RePayload,
) -> zbus::Result<()> {
    conn.emit_signal(
        Option::<BusName>::None,
        path,
        "org.freedesktop.portal.Request",
        "Response",
        &payload.into_zvariant(),
    )
}

pub enum RePayload {
    PickedFiles(Vec<PathBuf>),
    UserCancel,
}

impl RePayload {
    fn into_zvariant(
        self,
    ) -> zvariant::DynamicTuple<(u32, HashMap<String, zvariant::Value<'static>>)> {
        match self {
            Self::PickedFiles(path_bufs) => zvariant::DynamicTuple((
                0u32,
                HashMap::<String, zvariant::Value>::from_iter([(
                    "uris".into(),
                    path_bufs
                        .into_iter()
                        .map(|buf| format!("file:///{}", buf.display()))
                        .collect::<Vec<_>>()
                        .into(),
                )]),
            )),
            Self::UserCancel => {
                zvariant::DynamicTuple((1u32, HashMap::<String, zvariant::Value>::default()))
            }
        }
    }
}
