# Building for Windows

This guide shows how to build the simple_tip_prep example as a Windows executable that opens a console window and displays logs.

## Building on Linux for Windows (Cross-compilation)

### 1. Install Windows Target

```bash
# Add Windows target
rustup target add x86_64-pc-windows-gnu

# Install MinGW-w64 (Ubuntu/Debian)
sudo apt install gcc-mingw-w64-x86-64

# Or on other systems, install mingw-w64
```

### 2. Build the Executable

```bash
# Build release version for Windows
cargo build --example simple_tip_prep --target x86_64-pc-windows-gnu --release

# The executable will be created at:
# target/x86_64-pc-windows-gnu/release/examples/simple_tip_prep.exe
```

### 3. Alternative: Using MSVC Target (Requires Windows SDK)

```bash
# Add MSVC target
rustup target add x86_64-pc-windows-msvc

# Build with MSVC (requires Windows SDK)
cargo build --example simple_tip_prep --target x86_64-pc-windows-msvc --release
```

## Building on Windows

### Using Rust installed on Windows:

```cmd
# Build release version
cargo build --example simple_tip_prep --release

# The executable will be at:
# target/release/examples/simple_tip_prep.exe
```

## Windows Console Features

The Windows executable includes:

- **Automatic console allocation**: Opens a console window if launched from GUI
- **Console title**: Sets window title to "Rusty Tip Preparation Tool"
- **ANSI colors**: Enables colored log output on Windows 10+
- **Press to exit**: Waits for Enter key before closing (prevents window from disappearing)

## Usage

```cmd
# Run with default config
simple_tip_prep.exe

# Run with custom config
simple_tip_prep.exe --config config.toml

# Run with debug logging
simple_tip_prep.exe --log-level debug

# Run with custom config and debug logging
simple_tip_prep.exe --config my_config.toml --log-level debug
```

## Log Levels

Available log levels (from most to least verbose):
- `trace` - Very detailed debugging information
- `debug` - Debugging information
- `info` - General information (default)
- `warn` - Warning messages
- `error` - Error messages only

## Configuration

The tool will look for configuration files in this order:
1. File specified with `--config`
2. `config.toml` in current directory
3. `base_config.toml` in current directory
4. `examples/base_config.toml`
5. Built-in defaults

Environment variables can override any config setting:
```cmd
set RUSTY_TIP__NANONIS__HOST_IP=192.168.1.100
set RUSTY_TIP__LOGGING__LOG_LEVEL=debug
simple_tip_prep.exe
```

## Distribution

The resulting `.exe` file is a single executable that can be distributed to Windows machines without requiring Rust to be installed. Just ensure the target machine has the required Windows runtime libraries.