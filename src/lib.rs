use anyhow::Result;
use rcon::Connection;
use regex::Regex;
use std::io::ErrorKind;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration, Instant, interval};

/// Read a VarInt from the buffer, returning (value, bytes_read). Returns None if malformed
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

/// Waits for a full Minecraft LoginStart handshake (state = Login) on an existing listener,
/// not just a status ping. The listener should be bound once by the caller.
pub async fn wait_for_login(listener: &TcpListener) -> Result<()> {
    let addr = listener.local_addr()?;
    log::info!("Listening for login on {}", addr);

    loop {
        // 1) Accept new TCP connection, ignoring transient errors
        let (mut socket, peer) = match listener.accept().await {
            Ok(pair) => {
                log::info!("Incoming TCP connection from {}", pair.1);
                pair
            }
            Err(e) => {
                log::warn!("Failed to accept connection: {} (retrying)", e);
                continue;
            }
        };

        // 2) Read initial data, ignoring resets or immediate closes
        let mut buf = [0u8; 512];
        let n = match socket.read(&mut buf).await {
            Ok(0) => {
                log::debug!("Connection closed immediately by {}", peer);
                continue;
            }
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                log::debug!("Connection reset by peer {} (ignoring)", peer);
                continue;
            }
            Err(e) => {
                // Unexpected I/O error, propagate
                return Err(e.into());
            }
        };

        log::debug!("Received {} bytes: {:02X?}", n, &buf[..n]);

        // 3) Parse handshake packet (packet ID = 0, next_state = 2)
        // Skip packet length VarInt
        let (_pkt_len, off1) = match read_varint(&buf[..n]) {
            Some(v) => v,
            None => continue,
        };
        // Packet ID VarInt
        let (pkt_id, off2) = match read_varint(&buf[off1..n]) {
            Some(v) => v,
            None => continue,
        };
        if pkt_id != 0 {
            // not a handshake packet
            continue;
        }

        // Skip protocol version VarInt
        let mut offset = off1 + off2;
        let (_protocol_version, len) = match read_varint(&buf[offset..n]) {
            Some(v) => v,
            None => continue,
        };
        offset += len;

        // Read address length and skip the address string
        let (addr_len, len) = match read_varint(&buf[offset..n]) {
            Some(v) => v,
            None => continue,
        };
        if addr_len < 0 {
            continue;
        }
        offset += len + addr_len as usize;

        // Skip the port (2 bytes)
        offset += 2;

        // Finally read next_state VarInt
        if offset >= n {
            continue;
        }
        if let Some((next_state, _)) = read_varint(&buf[offset..n]) {
            if next_state == 2 {
                log::info!("Login handshake detected from {}", peer);
                break;
            } else {
                log::debug!("Status ping from {}, ignoring", peer);
            }
        }
    }

    Ok(())
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
