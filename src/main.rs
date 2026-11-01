#![forbid(unsafe_code)]
#![warn(clippy::pedantic, clippy::nursery)]

use {config::Config, std::error::Error};

mod config;
mod dbus;
mod window;

fn main() -> Result<(), Box<dyn Error>> {
    let (portal, recv) = dbus::make_file_portal();
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _join_handle = tokio_rt.spawn(async { dbus::make_connection(portal).await });
    let mut windows = Vec::new();
    let mut cfg = Config::load_or_default();
    loop {
        match recv.try_recv() {
            Ok(req) => {
                window::spawn_window(req, &mut windows, &cfg);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(e) => {
                eprintln!("Recv Error: {e}");
            }
        }
        window::update_windows(&mut windows, &mut cfg);
    }
}
