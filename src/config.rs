use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    config_directory_name: String,
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
            config_directory_name: "config".to_string(),
        }
    }
}

pub fn get_config() -> Config {
    let mut config = Config::default();

    // Search subdirectories for cfg.toml
    let mut old_config: Option<Config> = None;
    let mut old_config_dir: Option<String> = None;
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let config_file = path.join("cfg.toml");
                if config_file.exists() {
                    if let Ok(contents) = fs::read_to_string(&config_file) {
                        if let Ok(parsed_config) = toml::from_str::<Config>(&contents) {
                            old_config = Some(parsed_config);
                            old_config_dir = path.to_str().map(|s| s.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    // If an old config with a different directory name is found, migrate it
    if let (Some(old_cfg), Some(old_dir)) = (&old_config, &old_config_dir) {
        // Normalize directory names for comparison
        let old_dir_normalized = PathBuf::from(&old_dir)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&old_dir));

        let new_dir_normalized = PathBuf::from(&old_cfg.config_directory_name)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&old_cfg.config_directory_name));

        if old_dir_normalized != new_dir_normalized && Path::new(&old_dir_normalized).exists() {
            log::info!(
                "Found old configuration directory '{}'. Migrating to '{}'.",
                old_dir_normalized.display(),
                new_dir_normalized.display()
            );

            fs::rename(&old_dir_normalized, &new_dir_normalized)
                .expect("Failed to migrate config directory");
        }
    }

    if let Some(old_cfg) = old_config {
        config = old_cfg;
    }

    let config_dir = config.config_directory_name.as_str();
    let config_path = format!("{}/cfg.toml", config_dir);
    // Create config directory if it doesn't exist
    if !Path::new(config_dir).exists() {
        log::info!("No configuration directory found. Creating configuration directory.");
        fs::create_dir(config_dir).expect("Cannot create config directory");
    }

    match fs::read_to_string(&config_path) {
        Ok(contents) => config = toml::from_str::<Config>(&contents).unwrap_or_default(),
        Err(_) => {
            log::info!(
                "No configuration file found. Creating default configuration file at {}.",
                config_path
            );
            File::create(&config_path).expect("Cannot create config file");
        }
    };

    let icon_path = format!("{}/server-icon.png", config.config_directory_name);
    match resize_image_to_64x64(&icon_path) {
        Ok(resized_image) => {
            // Save resized image back to server-icon.png
            resized_image
                .save(&icon_path)
                .expect("Failed to save resized server-icon.png");

            config.server_icon = Some(convert_servericon_to_base64(&icon_path));
        }
        Err(_) => {
            log::info!(
                "No server-icon.png found in {}/ directory.",
                config.config_directory_name
            );
            config.server_icon = None;
        }
    };

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&config_path)
        .expect("Cannot open config file for writing");

    let toml_str = toml::to_string_pretty(&config).unwrap();
    file.write_all(toml_str.as_bytes())
        .expect("Cannot write to config file");
    return config;
}

fn resize_image_to_64x64(path: &str) -> Result<DynamicImage> {
    let img = image::open(path)?;
    let (width, height) = img.dimensions();
    if width == 64 && height == 64 {
        return Ok(img); // Return original image if size is already 64x64
    }
    return Ok(img.resize_exact(64, 64, FilterType::CatmullRom));
}

fn convert_servericon_to_base64(path: &str) -> String {
    let image_bytes = fs::read(path).expect("Failed to read server-icon.png");
    let image_base64 = general_purpose::STANDARD.encode(&image_bytes);
    return image_base64;
}
