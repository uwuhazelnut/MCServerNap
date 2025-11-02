use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Duration;

// Import core functions from the library crate
use mcservernap::config;
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
        .filter_level(log::LevelFilter::Debug) // !!! CHANGE THIS BACK TO INFO BEFORE RELEASE !!!
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
            let server_starting = Arc::new(AtomicBool::new(false));
            let app_config: config::Config = config::get_config();
            let listener = TcpListener::bind(addr).await?;

            let (tx, _) = tokio::sync::watch::channel(false);
            let atomic_tx = Arc::new(tx);

            log::info!("Listening for login on {}", addr);
            loop {
                // Bind listener every loop iteration because we drop listener inside the loop

                let (mut client_socket, peer) = listener.accept().await?;
                log::info!("Incoming TCP connection from {}", peer);

                let mut rx = atomic_tx.subscribe();
                let tx_clone = atomic_tx.clone();

                if !server_running.load(Ordering::SeqCst) {
                    match verify_handshake_packet(&mut client_socket, peer, &app_config).await {
                        Ok(true) => {
                            // Server is offline: notify player client
                            log::info!("Notifying {} (server offline)", peer);
                            if let Err(e) =
                                mcservernap::send_starting_message(client_socket, &app_config).await
                            {
                                log::warn!("Failed to notify {}: {}", peer, e);
                            }

                            let server_running_clone = server_running.clone();
                            tokio::spawn(async move {
                                while !*rx.borrow() {
                                    rx.changed().await.unwrap();
                                }

                                server_running_clone.store(true, Ordering::SeqCst);
                            });

                            // Launch server now
                            let dereferenced_tx = (*tx_clone).clone();
                            if !server_starting.load(Ordering::SeqCst) {
                                let mut child = launch_server(&cmd, &arg_slices)?;
                                server_starting.store(true, Ordering::SeqCst);

                                let rcon_addr_clone = rcon_addr.clone();
                                let rcon_pass_inner = rcon_pass_clone.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = idle_watchdog_rcon(
                                        &rcon_addr_clone,
                                        &rcon_pass_inner,
                                        Duration::from_secs(app_config.rcon_poll_interval), // check interval
                                        Duration::from_secs(app_config.rcon_idle_timeout), // idle timeout
                                        dereferenced_tx, // RCON connected transmitter
                                    )
                                    .await
                                    {
                                        log::error!("Idle watchdog error: {}", e);
                                    }
                                });

                                let server_running_clone_clone = server_running.clone();
                                tokio::spawn(async move {
                                    // Wait for server exit
                                    match child.wait().await {
                                        Ok(_) => log::info!("Server exited"),
                                        Err(e) => log::error!("Failed to wait: {:?}", e),
                                    }

                                    server_running_clone_clone.store(false, Ordering::SeqCst);
                                    log::info!(
                                        "Server stopped. Restarting listener for next connection..."
                                    );
                                });
                            }
                        }
                        Ok(false) => continue, // Not a login handshake, ignore
                        Err(_) => continue,    // Wait for next connection
                    }
                } else {
                    let mut server_socket = TcpStream::connect("127.0.0.1:25566").await?;
                    tokio::spawn(async move {
                        match tokio::io::copy_bidirectional(&mut client_socket, &mut server_socket)
                            .await
                        {
                            Ok((read, written)) => {
                                log::debug!(
                                    "Proxy successful: read {} bytes, wrote {}",
                                    read,
                                    written
                                );
                            }
                            Err(e) => {
                                log::error!("Proxy error: {:?}", e);
                            }
                        }
                    });
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
