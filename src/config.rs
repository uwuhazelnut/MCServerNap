use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub rcon_poll_interval: u64,
    pub rcon_idle_timeout: u64,
    pub motd_text: String,
    pub motd_color: String,
    pub motd_bold: bool,
    pub connection_msg_text: String,
    pub connection_msg_color: String,
    pub connection_msg_bold: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            rcon_poll_interval: 60,
            rcon_idle_timeout: 600,
            motd_text: "Napping... Join to start server".to_string(),
            motd_color: "aqua".to_string(),
            motd_bold: true,
            connection_msg_text: "Server is now starting up. Please wait and try again shortly..."
                .to_string(),
            connection_msg_color: "light_purple".to_string(),
            connection_msg_bold: true,
        }
    }
}

pub fn get_config() -> Config {
    let config_dir = "config";
    let config_path = "config/cfg.toml";
    // Create config directory if it doesn't exist
    if !Path::new(config_dir).exists() {
        log::info!("No configuration directory found. Creating configuration directory.");
        fs::create_dir(config_dir).expect("Cannot create config directory");
    }

    match fs::read_to_string(config_path) {
        Ok(contents) => return toml::from_str(&contents).unwrap_or_else(|_| Config::default()),
        Err(_) => {
            log::info!("No configuration file found. Creating default configuration file at {}.", config_path);
            let default_config = Config::default();
            let toml_str = toml::to_string_pretty(&default_config).unwrap();
            let mut file = File::create(config_path).expect("Cannot create config file");
            file.write_all(toml_str.as_bytes())
                .expect("Cannot write default config");
            return default_config;
        }
    };
}
