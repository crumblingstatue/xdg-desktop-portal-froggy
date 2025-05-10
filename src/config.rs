use {
    ron::ser::PrettyConfig,
    serde::{Deserialize, Serialize},
    std::error::Error,
};

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub file_dia_storage: egui_file_dialog::FileDialogStorage,
}

impl Config {
    pub fn load_or_default() -> Self {
        match std::fs::read_to_string(cfg_file()) {
            Ok(contents) => match ron::from_str(&contents) {
                Ok(this) => this,
                Err(e) => {
                    eprintln!("Failed to deserialize config: {e}");
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to load config: {e}");
                Self::default()
            }
        }
    }
    pub fn save(&self) -> Result<(), Box<dyn Error>> {
        std::fs::create_dir_all(cfg_dir())?;
        let out = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        std::fs::write(cfg_file(), out.as_bytes())?;
        Ok(())
    }
}

fn cfg_file() -> std::path::PathBuf {
    let cfg_dir = cfg_dir();
    cfg_dir.join("config.ron")
}

fn cfg_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap()
        .join("xdg-desktop-portal-froggy")
}
