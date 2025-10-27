pub mod config;

use crate::config::Config;
use anyhow::Result;
use rcon::Connection;
use regex::Regex;
use serde_json::json;
use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, Instant, interval};

/// Read a VarInt (Minecraft format) from the buffer, returning (value, bytes_read). Returns None if malformed
fn read_varint(buf: &[u8]) -> Option<(i32, usize)> {
    let mut num_read = 0;
    let mut result = 0i32;
    for &byte in buf.iter() {
        let val = (byte & 0x7F) as i32;
        result |= val << (7 * num_read);
        num_read += 1;
        if byte & 0x80 == 0 {
            return Some((result, num_read));
        }
        if num_read >= 5 {
            return None;
        }
    }
    None
}

// Write a VarInt (Minecraft format)
pub fn write_varint(mut val: i32, buf: &mut Vec<u8>) {
    loop {
        if (val & !0x7F) == 0 {
            buf.push(val as u8);
            return;
        } else {
            buf.push(((val & 0x7F) | 0x80) as u8);
            val >>= 7;
        }
    }
}

// Verifies a full Minecraft handshake on a single TcpStream.
pub async fn verify_handshake_packet(
    socket: &mut TcpStream,
    peer: SocketAddr,
    config: &Config,
) -> Result<bool> {
    // 1) Read initial data, ignoring resets or immediate closes
    let mut buf = [0u8; 512];
    let n = match socket.read(&mut buf).await {
        Ok(0) => {
            log::debug!("Connection closed immediately by {}", peer);
            return Ok(false);
        }
        Ok(n) => n,
        Err(e) if e.kind() == ErrorKind::ConnectionReset => {
            log::debug!("Connection reset by peer {} (ignoring)", peer);
            return Ok(false);
        }
        Err(e) => {
            // Unexpected I/O error, propagate
            return Err(e.into());
        }
    };

    log::debug!("Received {} bytes: {:02X?}", n, &buf[..n]);

    // 2) Parse handshake packet (packet ID = 0, next_state = 2)
    // More information on the handshake packet structure: https://minecraft.wiki/w/Java_Edition_protocol/Packets#Handshaking
    // Skip packet length VarInt
    let (_pkt_len, off1) = match read_varint(&buf[..n]) {
        Some(v) => v,
        None => return Ok(false),
    };
    // Packet ID VarInt
    let (pkt_id, off2) = match read_varint(&buf[off1..n]) {
        Some(v) => v,
        None => return Ok(false),
    };
    if pkt_id != 0 {
        // not a handshake packet
        return Ok(false);
    }

    // Skip protocol version VarInt
    let mut offset = off1 + off2;
    let (_protocol_version, len) = match read_varint(&buf[offset..n]) {
        Some(v) => v,
        None => return Ok(false),
    };
    offset += len;

    // Read address length and skip the address string
    let (addr_len, len) = match read_varint(&buf[offset..n]) {
        Some(v) => v,
        None => return Ok(false),
    };
    if addr_len < 0 {
        return Ok(false);
    }
    offset += len + addr_len as usize;

    // Skip the port (2 bytes)
    offset += 2;

    // Read next_state (intent) VarInt
    if offset >= n {
        return Ok(false);
    }
    if let Some((next_state, _)) = read_varint(&buf[offset..n]) {
        if next_state == 1 {
            // Status ping
            handle_status_ping(socket, &config).await?;
            return Ok(false);
        } else if next_state == 2 {
            // Login handshake
            log::info!("Login handshake detected from {}", peer);
            return Ok(true);
        } else {
            log::debug!("Unknown type of ping from {}, ignoring", peer);
        }
    }

    Ok(false)
}

/// Launches the Minecraft server process with given command.
/// On Windows, opens the batch/script in a new terminal window so logs stay visible
pub fn launch_server(command: &str, args: &[&str]) -> Result<std::process::Child> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(&["/C", "start", "", "/WAIT", command]);
        for &arg in args {
            cmd.arg(arg);
        }
        let child = cmd.spawn()?;
        log::info!("Launched server in new window: {} {:?}", command, args);
        Ok(child)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let child = std::process::Command::new(command).args(args).spawn()?;
        log::info!("Launched server: {} {:?}", command, args);
        Ok(child)
    }
}

/// Idle watchdog: polls the RCON `list` command every `poll_interval`.
/// If no players have been online for `timeout`, send `/stop` via RCON and exit
pub async fn idle_watchdog_rcon(
    rcon_addr: &str,
    rcon_pass: &str,
    poll_interval: Duration,
    timeout: Duration,
) -> Result<()> {
    log::info!(
        "Starting RCON idle watchdog: polling {} every {:?}",
        rcon_addr,
        poll_interval
    );
    let start = Instant::now();

    // Wait for RCON to become available
    let conn = loop {
        match Connection::<TcpStream>::connect(rcon_addr, rcon_pass).await {
            Ok(c) => break c,
            Err(err) if start.elapsed() <= Duration::from_secs(120) => {
                log::warn!("RCON connection failed ({}), retrying...", err);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(err) => {
                return Err(err.into());
            }
        }
    };

    let mut conn = conn;
    log::info!("Successfully connected to RCON at {}", rcon_addr);

    // Polling loop
    let player_count_re = Regex::new(r"There are (\d+) of a max").unwrap();
    let mut ticker = interval(poll_interval);
    let mut last_online = Instant::now();

    loop {
        ticker.tick().await;
        let response = conn.cmd("list").await?;
        log::info!("RCON list response: {}", response);

        let count = player_count_re
            .captures(&response)
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);

        if count > 0 {
            last_online = Instant::now();
        } else if last_online.elapsed() >= timeout {
            log::info!("No players for {:?}, stopping server...", timeout);
            let _ = conn.cmd("stop").await;
            break;
        }
    }
    Ok(())
}

/// Sends a single `/stop` command to the server via RCON and exits
pub async fn send_stop_command(rcon_addr: &str, rcon_pass: &str) -> Result<()> {
    log::info!(
        "Connecting to RCON at {} to send stop command...",
        rcon_addr
    );
    let mut conn = Connection::<TcpStream>::connect(rcon_addr, rcon_pass).await?;
    let _ = conn.cmd("stop").await?;
    log::info!("Stop command sent.");
    Ok(())
}

pub async fn send_starting_message(mut socket: TcpStream, config: &Config) -> Result<()> {
    let json_msg = json!({
        "text": config.connection_msg_text,
        "color": config.connection_msg_color,
        "bold": config.connection_msg_bold
    })
    .to_string();
    let mut packet_data = Vec::new();

    //Packet ID 0x00 (login disconnect)
    write_varint(0, &mut packet_data);

    write_varint(json_msg.len() as i32, &mut packet_data);
    packet_data.extend_from_slice(json_msg.as_bytes());

    let mut packet = Vec::new();
    write_varint(packet_data.len() as i32, &mut packet);
    packet.extend_from_slice(&packet_data);

    socket.write_all(&packet).await?;

    // Wait a short moment to let client consume data (required because otherwise client doesn't display json message)
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    socket.shutdown().await?;
    Ok(())
}

async fn handle_status_ping(socket: &mut TcpStream, config: &Config) -> Result<()> {
    // Read and discard the next packet (packet ID 0, status request)
    let mut buf = [0u8; 512];
    let _n = socket.read(&mut buf).await?;

    // Create custom MOTD JSON
    // Protocol is "an integer used to check for incompatibilities between the player's client and the server
    // they are trying to connect to.". 766 = Minecraft 1.20.5 (https://minecraft.fandom.com/wiki/Protocol_version)
    let motd_json = json!({
        "version": {
            "name": "MCServerNap (1.20.5)",
            "protocol": 766
        },
        "players": {
            "max": 0,
            "online": 0,
            "sample": []
        },
        "description": {
            "text": config.motd_text,
            "color": config.motd_color,
            "bold": config.motd_bold
        }
    })
    .to_string();

    // Create status response packet
    let mut data = Vec::new();
    // Packet ID = 0 (status response)
    write_varint(0, &mut data);
    write_varint(motd_json.len() as i32, &mut data);
    data.extend_from_slice(motd_json.as_bytes());

    let mut packet = Vec::new();
    write_varint(data.len() as i32, &mut packet);
    packet.extend_from_slice(&data);

    // Send to client
    socket.write_all(&packet).await?;
    socket.shutdown().await?;
    Ok(())
}
