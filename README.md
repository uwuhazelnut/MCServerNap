# MCServerNap

*A lightweight, serverless Minecraft server watcher and auto-starter.*

## Overview

`mcservernap` monitors incoming Minecraft client connections and automatically launches (and later stops) a local Minecraft server process via RCON. It enables you to avoid running your server 24/7 by:

* Listening for the first legitimate Minecraft **LoginStart** handshake.
* Spinning up the server process on-demand when a player attempts to join.
* Watching the server via RCON for player activity.
* Stopping the server after an idle timeout.

<img width="657" height="94" alt="screenshot of server browser view" src="https://github.com/user-attachments/assets/dae15e22-849e-4469-bae9-df17cc94636b" />
<img width="966" height="261" alt="screenshot of connect message" src="https://github.com/user-attachments/assets/ca128f11-5e7a-4666-a03c-6d56235385db" />


There is also a `stop` subcommand to immediately send a `/stop` command via RCON.

## Features

* **On-demand startup**: server only runs when a player actually joins.
* **Idle shutdown**: automatically stops server when no players remain for a set duration.
* **Cross-platform**: spawns a new terminal window on Windows, runs directly on Linux systems.
* **Extensible**: configure RCON settings, startup command and ports.

## Installation

1. Ensure you have Rust and Cargo installed (see [rustup.rs](https://rustup.rs)).
2. Clone this repository:

   ```bash
   git clone https://github.com/yourusername/MCServerNap.git
   cd MCServerNap
   ```
3. Build the binary:

   ```bash
   cargo build --release
   ```

   The executable can be found under `target/release/mcservernap.exe`
4. (Optional) If you wish to install globally:

   ```bash
   cargo install --path .
   ```

## Usage

```bash
mcservernap <COMMAND> [OPTIONS]
```

### Subcommands

* `listen` — Listen for incoming connections and start the server on first join.
* `stop` — Immediately send a `/stop` command via RCON to shut down an already-running server.

### `listen` Options

| Option          | Description                                                            | Required |
| --------------- | ---------------------------------------------------------------------- | -------- |
| `host`          | Host or IP to bind (e.g. `0.0.0.0`)                                    | Yes      |
| `port`          | Port to listen on for Minecraft clients                                | Yes      |
| `cmd`           | Command or script to launch the Minecraft server                       | Yes      |
| `args...`       | Arguments passed to the server command                                 | No       |
| `--server-port` | Port of the actual Minecraft Server that users will get forwarded to   | Yes      |
| `--rcon-port`   | Port for the server’s RCON interface                                   | Yes      |
| `--rcon-pass`   | Password for RCON authentication                                       | Yes      |

> [!NOTE]
> The port of the Minecraft server does not require port forwarding, only the port of this application.

#### Example

```bash
mcservernap listen 0.0.0.0 25565 java -Xmx5G -Xms5G -jar server.jar nogui --server-port 25566 --rcon-port 25575 --rcon-pass rconpasswordmeow
```

#### Script Example

```bash
mcservernap listen 0.0.0.0 25565 "C:\path\to\your\script\start_server.bat" --server-port 25566 --rcon-port 25575 --rcon-pass rconpasswordmeow
```
**IMPORTANT: When using a script, make sure the script closes its window at the end of the script (Windows .bat example: `exit`), or else this application won't detect that the Minecraft server process has shut down!**

Once a client sends a LoginStart packet, the tool:

1. Drops the listener and launches your server command.
2. Starts an **idle watchdog** task that polls RCON according to `rcon_poll_interval`.
3. If no players remain for the defined amount of `rcon_idle_timeout` time, sends `/stop` and exits.

### `stop` Options

| Option        | Description                          | Required |
| ------------- | ------------------------------------ | -------- |
| `--rcon-port` | Port for the server’s RCON interface | Yes      |
| `--rcon-pass` | Password for RCON authentication     | Yes      |

#### Example

```bash
mcservernap stop --rcon-port 25575 --rcon-pass rconpasswordmeow
```

This immediately connects via RCON and sends the `/stop` command.

## Configuration & Environment

### **Logging**: Controlled via entry point of `main()`:

```rust
env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info) // Change this LevelFilter to change logging level (e.g. Debug)
        .init();
```
You need to rebuild the project for the change to take effect.

### The **configuration** will be generated on first time usage of this application under `config/cfg.toml`
Configuration Options:
* **Timeouts & Intervals**: set via `rcon_idle_timeout` and `rcon_poll_interval` in <ins>seconds</ins>
* **Message of the day (MOTD)**: The message shown to the user in the server browser menu. set via `motd_text`, `motd_color` and `motd_bold`
* **Connection Message**: The message shown to the user when they try to connect. Set via `connection_msg_text`, `connection_msg_color` and `connection_msg_bold`
* **Server Icon**: The icon of the server within the server browser menu. Set by inserting a `.png` file in the `config/` folder with the name `server-icon.png`. The image must be 64x64 pixels big. If it's not, this application will automatically resize the image to meet this requirement
* **Configuration Directory**: The location of the `cfg.toml` can be changed from the standard `config/` directory by editing the value of `config_directory_name`. This will delete the previous directory and move the files to the new one

## Contributing

Contributions are welcome! Feel free to open issues or pull requests to:

* Support TLS or SSH tunnels for RCON

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.
