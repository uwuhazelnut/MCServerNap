use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::Duration;

// Import core functions from the library crate
use mcservernap::{idle_watchdog_rcon, launch_server, send_stop_command, verify_handshake_packet};

/// "Serverless" Minecraft Server Watcher
#[derive(Parser)]
#[command(name = "mcservernap")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Listen on port and start server on first actual join
    Listen {
        /// Host/IP to bind
        host: String,
        /// Port to listen on
        port: u16,
        /// Command to launch (e.g. 'java' or path to start script)
        cmd: String,
        /// Arguments for the command (pass all Java/batch args here)
        #[arg(num_args(0..))]
        args: Vec<String>,
        /// RCON port (use --rcon-port)
        #[arg(long)]
        rcon_port: u16,
        /// RCON password (use --rcon-pass)
        #[arg(long)]
        rcon_pass: String,
    },
    /// Immediately stop the Minecraft server via RCON
    Stop {
        /// RCON port
        #[arg(long)]
        rcon_port: u16,
        /// RCON password
        #[arg(long)]
        rcon_pass: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialise logger
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Listen {
            host,
            port,
            cmd,
            args,
            rcon_port,
            rcon_pass,
        } => {
            let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
            let arg_slices: Vec<&str> = args.iter().map(String::as_str).collect();
            let rcon_addr = format!("127.0.0.1:{}", rcon_port);
            let rcon_pass_clone = rcon_pass.clone();
            let server_running = Arc::new(AtomicBool::new(false));

            loop {
                // Bind listener every loop iteration because we drop listener inside the loop
                let listener = TcpListener::bind(addr).await?;
                log::info!("Listening for login on {}", addr);

                let (mut socket, peer) = listener.accept().await?;
                log::info!("Incoming TCP connection from {}", peer);

                match verify_handshake_packet(&mut socket, peer).await {
                    Ok(true) => {
                        if !server_running.load(Ordering::SeqCst) {
                            // Server is offline: notify player client
                            log::info!("Notifying {} (server offline)", peer);
                            if let Err(e) = mcservernap::send_starting_message(socket).await {
                                log::warn!("Failed to notify {}: {}", peer, e);
                            }
                            // Launch server now
                            server_running.store(true, Ordering::SeqCst);
                            drop(listener); // To-Do: DELAY LISTENER DROP UNTIL SERVER HAS STARTED OR ELSE THE USER GETS A CONNECTION ERROR IF THEY RECONNECT TOO EARLY
                            let mut child = launch_server(&cmd, &arg_slices)?;

                            let rcon_addr_clone = rcon_addr.clone();
                            let rcon_pass_inner = rcon_pass_clone.clone();
                            tokio::spawn(async move {
                                if let Err(e) = idle_watchdog_rcon(
                                    &rcon_addr_clone,
                                    &rcon_pass_inner,
                                    Duration::from_secs(60), // 1 minute check interval
                                    Duration::from_secs(600), // 10 minutes idle timeout
                                )
                                .await
                                {
                                    log::error!("Idle watchdog error: {}", e);
                                }
                            });

                            // Wait for server exit
                            match child.wait() {
                                Ok(_) => log::info!("Server exited"),
                                Err(e) => log::error!("Failed to wait: {:?}", e),
                            }
                            server_running.store(false, Ordering::SeqCst);
                            log::info!(
                                "Server stopped. Restarting listener for next connection..."
                            );
                        } else {
                            // Server is running:
                            // (Maybe forward the raw TCP to the actual server, to proxy direct connections)
                            log::info!("Server running; player client should retry.");
                            // Close socket if not proxying, or handle connection if proxying.
                            socket.shutdown().await?;
                        }
                    }
                    Ok(false) => continue, // Not a login handshake, ignore
                    Err(_) => continue,    // Wait for next connection
                }
            }
        }
        Commands::Stop {
            rcon_port,
            rcon_pass,
        } => {
            let rcon_addr = format!("127.0.0.1:{}", rcon_port);
            send_stop_command(&rcon_addr, &rcon_pass).await?;
        }
    }

    // We never break from the Listen loop, but this satisfies the return type
    Ok(())
}
