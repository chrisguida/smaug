#!/bin/sh

set -e

cd /tmp/
wget "https://github.com/ElementsProject/lightning/releases/download/v${CLN_VERSION}/clightning-v${CLN_VERSION}-Ubuntu-22.04.tar.xz"
sudo tar -xvf clightning-v${CLN_VERSION}-Ubuntu-22.04.tar.xz -C /usr/local --strip-components=2
