#!/bin/sh

set -e

cd /tmp/
wget "https://github.com/ElementsProject/lightning/releases/download/v${CLN_VERSION}/clightning-v${CLN_VERSION}-Ubuntu-22.04.tar.xz"
tar -xvf clightning-v${CLN_VERSION}-Ubuntu-22.04.tar.xz --strip-components=2
sudo mv ./usr/bin/* /usr/local/bin
