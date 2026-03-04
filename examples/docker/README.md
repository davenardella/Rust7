Docker & Docker Compose Setup Guide
===================================

Before reading this documentation, many thanks to [SoftPlc](https://github.com/fbarresi/SoftPlc) for providing a PLC in a OCI format!

---

This guide explains:

1.  How to install **Docker**

2.  How to start a project using a `docker-compose.yml` file

* * * * *

Quick Start
===================================

1. Start the PLC in Docker:

```shell
docker compose up -d
```

2. Create a second Datablock beside the default Datablock:

```shell
curl -X 'POST' \
  'http://localhost:8080/api/DataBlocks?id=2&size=1024' \
  -H 'accept: */*' \
  -d ''
```

3. Run the Rust examples:
```shell
task run
```

1\. Installing Docker
=====================

🐧 Linux (Ubuntu/Debian)
------------------------

### Step 1: Update packages

```shell
sudo apt update
sudo apt upgrade -y
```

### Step 2: Install required dependencies

```shell
sudo apt install ca-certificates curl gnupg -y
```

### Step 3: Add Docker's official GPG key

```shell
sudo install -m 0755 -d /etc/apt/keyrings\
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | \\
sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg\
sudo chmod a+r /etc/apt/keyrings/docker.gpg
```

### Step 4: Add repository

```shell
echo \
  "deb [arch=$(dpkg --print-architecture) \
  signed-by=/etc/apt/keyrings/docker.gpg] \
  https://download.docker.com/linux/ubuntu \
  $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
  sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
```

### Step 5: Install Docker Engine

```shell
sudo apt update\
sudo apt install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin -y
```

### Step 6: Verify installation

```shell
docker --version
```

Optional (run without sudo):

```shell
sudo usermod -aG docker $USER
```

Then log out and log back in.

* * * * *

🍎 macOS
--------

1.  Download Docker Desktop from the official website.

2.  Install the `.dmg` file.

3.  Start Docker Desktop.

4.  Verify:

```shell
docker --version
```

* * * * *

🪟 Windows
----------

1.  Install Docker Desktop for Windows.

2.  Enable WSL2 (if prompted).

3.  Restart your system.

4.  Verify in PowerShell:

```shell
docker --version
```

* * * * *

2\. Starting a Docker Compose Project
=====================================

Make sure you are inside the directory that contains the:

```shell
docker-compose.yml
```

* * * * *

Step 1: Build and start containers
----------------------------------

docker compose up --build

* * * * *

Step 2: Run in background (detached mode)
-----------------------------------------

```shell
docker compose up -d
```

* * * * *

Step 3: Stop containers
-----------------------

```shell
docker compose down
```

* * * * *

Useful Commands
---------------

### View running containers

```shell
docker ps
```

### View logs

```shell
docker compose logs
```

Follow logs live:

```shell
docker compose logs -f
```
