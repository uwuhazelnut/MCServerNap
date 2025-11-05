use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::Duration;

// Import core functions from the library crate
use mcservernap::config;
use mcservernap::{
    ServerState, idle_watchdog_rcon, launch_server, send_stop_command, verify_handshake_packet,
};

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
        /// Minecraft server port (use --server-port)
        #[arg(long)]
        server_port: u16,
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
            server_port,
            rcon_port,
            rcon_pass,
        } => {
            let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
            let arg_slices: Vec<&str> = args.iter().map(String::as_str).collect();
            let rcon_addr = format!("127.0.0.1:{}", rcon_port);
            let rcon_pass_clone = rcon_pass.clone();

            let server_state = Arc::new(Mutex::new(ServerState::Stopped));
            let app_config: config::Config = config::get_config();
            let listener = TcpListener::bind(addr).await?;

            log::info!("Listening for login on {}", addr);
            loop {
                let (mut client_socket, peer) = listener.accept().await?;
                log::info!("Incoming TCP connection from {}", peer);
                let server_state_clone = server_state.clone();

                let client_handled = {
                    // Scoped to hold the Mutex lock only while checking and possibly updating state
                    let mut state_guard = server_state_clone.lock().await;

                    match *state_guard {
                        ServerState::Stopped => {
                            // Start the server and RCON watchdog
                            match verify_handshake_packet(&mut client_socket, peer, &app_config)
                                .await
                            {
                                Ok(true) => {
                                    if let Err(e) = mcservernap::send_starting_message(
                                        client_socket,
                                        &app_config,
                                    )
                                    .await
                                    {
                                        log::warn!("Failed to notify {}: {}", peer, e);
                                    }

                                    // Transition to starting state
                                    *state_guard = ServerState::Starting;
                                    log::debug!("Server state set to Starting in main()");

                                    let mut child = launch_server(&cmd, &arg_slices)?;

                                    let rcon_addr_clone = rcon_addr.clone();
                                    let rcon_pass_inner = rcon_pass_clone.clone();
                                    let server_state_for_rcon_watchdog = server_state_clone.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = idle_watchdog_rcon(
                                            &rcon_addr_clone,
                                            &rcon_pass_inner,
                                            Duration::from_secs(app_config.rcon_poll_interval), // check interval
                                            Duration::from_secs(app_config.rcon_idle_timeout), // idle timeout
                                            server_state_for_rcon_watchdog,
                                        )
                                        .await
                                        {
                                            log::error!("Idle watchdog error: {}", e);
                                        }
                                    });

                                    let server_state_for_server_exit = server_state_clone.clone();
                                    tokio::spawn(async move {
                                        // Wait for server exit
                                        match child.wait().await {
                                            Ok(_) => (),
                                            Err(e) => log::error!(
                                                "Failed to wait for server exit: {:?}",
                                                e
                                            ),
                                        }

                                        let mut state = server_state_for_server_exit.lock().await;
                                        *state = ServerState::Stopped;
                                        log::debug!(
                                            "Server state set to Stopped after server exit in main()"
                                        );
                                        log::info!(
                                            "Server stopped. Restarting listener for next connection..."
                                        );
                                    });

                                    true
                                }
                                Ok(false) => false, // Not a login handshake, ignore
                                Err(_) => false,    // Wait for next connection
                            }
                        }
                        ServerState::Starting => {
                            // Keep notifying the player client that the server is starting
                            match verify_handshake_packet(&mut client_socket, peer, &app_config)
                                .await
                            {
                                Ok(true) => {
                                    if let Err(e) = mcservernap::send_starting_message(
                                        client_socket,
                                        &app_config,
                                    )
                                    .await
                                    {
                                        log::warn!(
                                            "Failed to notify {} while starting server: {}",
                                            peer,
                                            e
                                        );
                                    }

                                    true
                                }
                                Ok(false) => false,
                                Err(_) => false,
                            }
                        }
                        ServerState::Running => {
                            // Server is running: proxy connection to actual Minecraft server
                            log::info!("Proxying connection for {}", peer);
                            tokio::spawn(async move {
                                let server_addr = format!("127.0.0.1:{}", server_port);
                                match TcpStream::connect(server_addr).await {
                                    Ok(mut server_socket) => {
                                        match tokio::io::copy_bidirectional(
                                            &mut client_socket,
                                            &mut server_socket,
                                        )
                                        .await
                                        {
                                            Ok((read, written)) => {
                                                log::debug!(
                                                    "Proxy successful for {}: read {} bytes, wrote {}",
                                                    peer,
                                                    read,
                                                    written
                                                );
                                            }
                                            Err(e) => {
                                                log::error!("Proxy error for {}: {:?}", peer, e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "Failed to connect to Minecraft server for {}: {:?}",
                                            peer,
                                            e
                                        );
                                    }
                                }
                            });
                            true
                        }
                    }
                };

                if !client_handled {
                    // Connection ignored, just drop socket and continue accepting
                    log::debug!(
                        "Connection from {} ignored (not login handshake or not handled)",
                        peer
                    );
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
