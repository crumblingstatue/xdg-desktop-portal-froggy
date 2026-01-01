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
pub struct Req {
    pub title: String,
    pub path: ObjectPath<'static>,
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FilePortal {
    async fn open_file(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
        _parent_window: &str,
        title: &str,
        options: HashMap<&str, zvariant::Value<'_>>,
    ) -> ObjectPath<'_> {
        let sender = hdr.sender().unwrap();
        let mut sender = sender.to_string();
        sender = sender.strip_prefix(":").unwrap().replace('.', "_");

        let token: &str = match options.get("handle_token") {
            Some(zvariant::Value::Str(val)) => val,
            _ => panic!("Oh crap"),
        };
        let path = ObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .unwrap();
        server.at(&path, RequestPortalFacade).await.unwrap();
        self.sender
            .send(Req {
                title: title.to_owned(),
                path: path.clone(),
            })
            .unwrap();
        path
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
