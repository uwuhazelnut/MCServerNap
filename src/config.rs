use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub rcon_poll_interval: u64,
    pub rcon_idle_timeout: u64,
    pub motd_text: String,
    pub motd_color: String,
    pub motd_bold: bool,
    pub server_icon: Option<String>,
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
            server_icon: None,
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

    let mut config = Config::default();

    match fs::read_to_string(config_path) {
        Ok(contents) => config = toml::from_str(&contents).unwrap_or_else(|_| Config::default()),
        Err(_) => {
            log::info!(
                "No configuration file found. Creating default configuration file at {}.",
                config_path
            );
            File::create(config_path).expect("Cannot create config file");
        }
    };

    match resize_image_to_64x64() {
        Ok(resized_image) => {
            // Save resized image back to server-icon.png
            resized_image
                .save("config/server-icon.png")
                .expect("Failed to save resized server-icon.png");

            config.server_icon = Some(convert_servericon_to_base64());
        }
        Err(_) => {
            log::info!("No server-icon.png found in config/ directory.");
            config.server_icon = None;
        }
    };

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(config_path)
        .expect("Cannot open config file for writing");

    let toml_str = toml::to_string_pretty(&config).unwrap();
    file.write_all(toml_str.as_bytes())
        .expect("Cannot write to config file");
    return config;
}

fn resize_image_to_64x64() -> Result<DynamicImage> {
    let img = image::open("config/server-icon.png")?;
    let (width, height) = img.dimensions();
    if width == 64 && height == 64 {
        return Ok(img); // Return original image if size is already 64x64
    }
    return Ok(img.resize_exact(64, 64, FilterType::CatmullRom));
}

fn convert_servericon_to_base64() -> String {
    let image_bytes = fs::read("config/server-icon.png").expect("Failed to read server-icon.png");
    let image_base64 = general_purpose::STANDARD.encode(&image_bytes);
    return image_base64;
}
