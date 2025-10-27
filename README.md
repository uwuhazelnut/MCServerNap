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
* **Cross-platform**: spawns a new terminal window on Windows, runs directly on Linux systems (app not tested on Linux yet).
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

| Option        | Description                                      | Required |
| ------------- | ------------------------------------------------ | -------- |
| `host`        | Host or IP to bind (e.g. `0.0.0.0`)              | Yes      |
| `port`        | Port to listen on for Minecraft clients          | Yes      |
| `cmd`         | Command or script to launch the Minecraft server | Yes      |
| `args...`     | Arguments passed to the server command           | No       |
| `--rcon-port` | Port for the server’s RCON interface             | Yes      |
| `--rcon-pass` | Password for RCON authentication                 | Yes      |

#### Example

```bash
mcservernap listen 0.0.0.0 25565 java -Xmx5G -Xms5G -jar server.jar nogui --rcon-port 25575 --rcon-pass rconpasswordmeow
```

#### Script Example

```bash
mcservernap listen 0.0.0.0 25565 "C:\path\to\your\script\start_server.bat" --rcon-port 25575 --rcon-pass rconpasswordmeow
```
**IMPORTANT: When using a script, make sure the script closes its window at the end of the script (Windows .bat example: `exit`), or else this application won't detect that the Minecraft server process has shut down!**

Once a client sends a LoginStart packet, the tool:

1. Drops the listener and launches your server command.
2. Starts an **idle watchdog** task that polls RCON every 60 seconds.
3. If no players remain for 10 minutes, sends `/stop` and exits.

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

* **Logging**: Controlled via `RUST_LOG`. For example:

  ```bash
  RUST_LOG=info mcservernap listen ...
  ```
* **Timeouts & Intervals**: Currently hard-coded in `main.rs` as 60s poll interval and 600s idle timeout. To customize, modify and rebuild the source.

## Contributing

Contributions are welcome! Feel free to open issues or pull requests to:

* Support TLS or SSH tunnels for RCON
* Confirm Linux compatibility

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.
