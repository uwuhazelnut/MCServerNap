use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::Duration;

// Import core functions from the library crate
use mcservernap::config;
use mcservernap::{
    ServerState, idle_watchdog_rcon, launch_server, preserialized_packets::PreserializedPackets,
    send_stop_command, verify_handshake_packet,
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
            let rcon_addr = Arc::new(format!("127.0.0.1:{}", rcon_port));
            let rcon_pass = Arc::new(rcon_pass);

            let server_state = Arc::new(Mutex::new(ServerState::Stopped));
            let app_config: config::Config = config::get_config();
            let preserialized_packets = PreserializedPackets::new(&app_config);
            let listener = TcpListener::bind(addr).await?;

            log::info!("Listening for login on {}", addr);

            // Clone handles for shutdown handler
            let rcon_addr_shutdown = rcon_addr.clone();
            let rcon_pass_shutdown = rcon_pass.clone();
            let server_state_shutdown = server_state.clone();

            tokio::select! {
                _ = main_loop(
                    listener,
                    cmd,
                    args,
                    server_port,
                    rcon_addr,
                    rcon_pass,
                    server_state,
                    app_config,
                    preserialized_packets
                ) => {},
                _ = tokio::signal::ctrl_c() => {
                    log::info!("Shutdown signal received (Ctrl+C)");

                    // Check if server is running and send stop command
                    let mut state_guard = match tokio::time::timeout(Duration::from_secs(5), server_state_shutdown.lock()).await {
                        Ok(guard) => guard,
                        Err(_) => {
                            log::error!("Deadlock detected! Failed to acquire state lock");
                            // Panicking here since we can't safely proceed
                            panic!("State lock timeout - possible deadlock");
                        }
                    };

                    if *state_guard == ServerState::Running {
                        log::info!("Stopping Minecraft server gracefully...");
                        match state_guard.switch_to(ServerState::Stopped) {
                            Ok(_) => (),
                            Err(e) => log::error!("{}", e),
                        }
                        drop(state_guard); // Release Mutex lock before RCON call

                        if let Err(e) = send_stop_command(&rcon_addr_shutdown, &rcon_pass_shutdown).await {
                            log::error!("Failed to send stop command: {}", e);
                        } else {
                            // Give server time to stop
                            tokio::time::sleep(Duration::from_secs(10)).await;
                        }
                    }
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

    Ok(())
}

async fn main_loop(
    listener: TcpListener,
    cmd: String,
    args: Vec<String>,
    server_port: u16,
    rcon_addr: Arc<String>,
    rcon_pass: Arc<String>,
    server_state: Arc<Mutex<ServerState>>,
    app_config: config::Config,
    preserialized_packets: PreserializedPackets,
) -> Result<()> {
    let arg_slices: Vec<&str> = args.iter().map(String::as_str).collect();

    loop {
        log::info!("Listening...");

        match listener.accept().await {
            Ok((mut client_socket, peer)) => {
                client_socket.set_nodelay(true)?;
                log::info!("Incoming TCP connection from {}", peer);

                let client_handled = {
                    // Scoped to hold the Mutex lock only while checking and possibly updating state

                    let mut state_guard =
                        match tokio::time::timeout(Duration::from_secs(5), server_state.lock())
                            .await
                        {
                            Ok(guard) => guard,
                            Err(_) => {
                                log::error!("Deadlock detected! Failed to acquire state lock");
                                panic!("State lock timeout - possible deadlock");
                            }
                        };

                    match *state_guard {
                        ServerState::Stopped => {
                            // Start the server and RCON watchdog
                            match verify_handshake_packet(
                                &mut client_socket,
                                peer,
                                &preserialized_packets,
                            )
                            .await
                            {
                                Ok(true) => {
                                    if let Err(e) = mcservernap::send_starting_message(
                                        client_socket,
                                        &preserialized_packets,
                                    )
                                    .await
                                    {
                                        log::warn!("Failed to notify {}: {}", peer, e);
                                    }

                                    // Transition to starting state
                                    match state_guard.switch_to(ServerState::Starting) {
                                        Ok(_) => (),
                                        Err(e) => log::error!("{}", e),
                                    }

                                    let mut child = launch_server(&cmd, &arg_slices)?;

                                    let rcon_addr_clone = rcon_addr.clone();
                                    let rcon_pass_clone = rcon_pass.clone();
                                    let server_state_for_rcon_watchdog = server_state.clone();
                                    let rcon_watchdog_handle = tokio::spawn(async move {
                                        if let Err(e) = idle_watchdog_rcon(
                                            &rcon_addr_clone,
                                            &rcon_pass_clone,
                                            Duration::from_secs(app_config.rcon_poll_interval), // check interval
                                            Duration::from_secs(app_config.rcon_idle_timeout), // idle timeout
                                            server_state_for_rcon_watchdog,
                                        )
                                        .await
                                        {
                                            log::error!("Idle watchdog error: {}", e);
                                        }
                                    });

                                    let server_state_for_server_exit = server_state.clone();
                                    tokio::spawn(async move {
                                        // Wait for server exit
                                        match child.wait().await {
                                            Ok(_) => (),
                                            Err(e) => {
                                                log::error!(
                                                    "Failed to wait for server exit: {:?}",
                                                    e
                                                )
                                            }
                                        }

                                        rcon_watchdog_handle.abort();
                                        log::info!("RCON watchdog aborted");

                                        {
                                            let mut state = match tokio::time::timeout(
                                                Duration::from_secs(5),
                                                server_state_for_server_exit.lock(),
                                            )
                                            .await
                                            {
                                                Ok(guard) => guard,
                                                Err(_) => {
                                                    log::error!(
                                                        "Deadlock detected! Failed to acquire state lock"
                                                    );
                                                    panic!(
                                                        "State lock timeout - possible deadlock"
                                                    );
                                                }
                                            };
                                            match state.switch_to(ServerState::Stopped) {
                                                Ok(_) => (),
                                                Err(e) => log::error!("{}", e),
                                            }
                                        }
                                        log::info!("Server stopped.");
                                    });

                                    true
                                }
                                Ok(false) => false, // Not a login handshake, ignore
                                Err(_) => false,    // Wait for next connection
                            }
                        }
                        ServerState::Starting => {
                            // Keep notifying the player client that the server is starting
                            match verify_handshake_packet(
                                &mut client_socket,
                                peer,
                                &preserialized_packets,
                            )
                            .await
                            {
                                Ok(true) => {
                                    if let Err(e) = mcservernap::send_starting_message(
                                        client_socket,
                                        &preserialized_packets,
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
                                        server_socket.set_nodelay(true).unwrap();
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

                                        // Attempt graceful shutdown of sockets
                                        if let Err(e) = client_socket.shutdown().await {
                                            log::warn!(
                                                "Failed to shutdown client socket for {}: {:?}",
                                                peer,
                                                e
                                            );
                                        }
                                        if let Err(e) = server_socket.shutdown().await {
                                            log::warn!(
                                                "Failed to shutdown server socket for {}: {:?}",
                                                peer,
                                                e
                                            );
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
            Err(e) => {
                log::error!("Failed to accept connection: {:?}", e);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        }
    }
}
