# Quick Share

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A lightweight command-line tool to quickly share a single file over HTTP. It automatically generates download links (curl/wget) for the receiver and shuts down after usage limits are met.

## 🚀 Quick Start

Get started instantly with a single command.

**Linux / macOS**
```bash
curl -fsSL https://github.com/Newbluecake/quick-share/raw/master/install.sh | bash
```

**Windows (PowerShell)**
```powershell
iwr -useb https://github.com/Newbluecake/quick-share/raw/master/install.ps1 | iex
```

## ✨ Features

- **⚡ Instant Sharing**: Share a file with one command. No configuration needed.
- **🌐 Auto IP Detection**: Automatically finds your machine's LAN IP address.
- **🔒 Secure**: Exposes *only* the specific file you chose. No directory access.
- **⏳ Auto-Stop**: Server automatically stops after a set number of downloads or time limit.
- **📋 Ready-to-Use Links**: Generates `curl` and `wget` commands for easy copying.
- **📊 Live Monitoring**: Shows real-time download progress and logs.

## 📦 Installation

### Linux & macOS

The automatic installer uses pip to install Quick Share from GitHub. Python 3.8+ is required.

```bash
curl -fsSL https://github.com/Newbluecake/quick-share/raw/master/install.sh | bash
```

Or install directly with pip:
```bash
pip install git+https://github.com/Newbluecake/quick-share.git
```

### Windows

The installation script downloads `quick-share.exe` and adds it to your User PATH. No Python required.

1. Open PowerShell.
2. Run the following command:
   ```powershell
   iwr -useb https://github.com/Newbluecake/quick-share/raw/master/install.ps1 | iex
   ```
3. Restart your terminal to refresh the PATH.

Alternatively, if you have Python installed:
```powershell
pip install git+https://github.com/Newbluecake/quick-share.git
```

## 💡 Usage

### Basic Sharing
Share a file with default settings (Max 10 downloads, 5 minutes timeout):

```bash
quick-share document.pdf
```

*Output example:*
```text
Sharing: document.pdf
Size: 2.5 MB
--------------------------------------------------
Download Link: http://192.168.1.10:8000/document.pdf

Command for receiver:
  wget http://192.168.1.10:8000/document.pdf
  curl -O http://192.168.1.10:8000/document.pdf
--------------------------------------------------
Limits: 10 downloads or 5m timeout
Press Ctrl+C to stop sharing manually
```

### Custom Limits
Share a file allowing only **3 downloads** and keep server alive for **10 minutes**:

```bash
quick-share data.zip -n 3 -t 10m
```

### Custom Port
Share using a specific port (e.g., 9090):

```bash
quick-share image.png -p 9090
```

### Full Options

```text
usage: quick-share [-h] [-p PORT] [-n MAX_DOWNLOADS] [-t TIMEOUT] file_path

positional arguments:
  file_path             Path to the file to share

optional arguments:
  -h, --help            show this help message and exit
  -p PORT, --port PORT  Port to listen on (1024-65535)
  -n MAX_DOWNLOADS      Maximum number of downloads allowed (default: 10)
  -t TIMEOUT            Timeout duration (e.g., 30s, 5m, 1h) (default: 5m)
```

## 🛠️ Development

Instructions for building from source or contributing.

### Prerequisites
- Python 3.8+
- pip

### Build from Source

1. **Clone the repository**
   ```bash
   git clone https://github.com/Newbluecake/quick-share.git
   cd quick-share
   ```

2. **Install dependencies**
   ```bash
   pip install -r requirements.txt
   pip install -r requirements-dev.txt
   ```

3. **Run tests**
   ```bash
   pytest
   ```

4. **Build executable**
   Use the provided build script to create a standalone binary:
   ```bash
   ./build.sh
   ```
   The executable will be generated in the `dist/` directory.

## License

MIT
