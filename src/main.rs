use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::time::Duration;

// Import core functions from the library crate
use mcservernap::{idle_watchdog_rcon, launch_server, send_stop_command, wait_for_login};

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

            loop {
                // Bind listener fresh every loop iteration
                let listener = TcpListener::bind(addr).await?;

                log::info!("Waiting for player login to start on {}", addr);
                wait_for_login(&listener).await?;

                // Drop listener to free port before starting server
                drop(listener);

                log::info!("Login detected, launching server...");
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

                match child.wait() {
                    Ok(status) => log::info!("Server exited with status {:?}", status),
                    Err(e) => log::error!("Failed to wait for server: {:?}", e),
                }

                log::info!("Server stopped. Restarting listener for next connection...");
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
