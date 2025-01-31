#!/bin/sh

set -e

# Update package lists and install dependencies
sudo apt-get update
sudo apt-get install -y wget tar

# Fetch the latest release tag from GitHub
LATEST_TAG=$(wget -qO- https://api.github.com/repos/ElementsProject/lightning/releases/latest | grep '"tag_name"' | cut -d '"' -f 4)

# Download and extract the latest release binary
wget -q https://github.com/ElementsProject/lightning/releases/download/$LATEST_TAG/clightning-$LATEST_TAG-Ubuntu-24.04-amd64.tar.xz
sudo tar -xvf clightning-$LATEST_TAG-Ubuntu-24.04-amd64.tar.xz -C /usr/local --strip-components=2

# Clean up
test -f clightning-$LATEST_TAG-Ubuntu-24.04-amd64.tar.xz && rm clightning-$LATEST_TAG-Ubuntu-24.04-amd64.tar.xz

echo "c-lightning $LATEST_TAG installed successfully."
