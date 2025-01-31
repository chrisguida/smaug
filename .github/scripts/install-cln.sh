#!/bin/sh

set -e

sudo apt-get update
sudo apt-get install -y \
  autoconf automake build-essential git libtool libsqlite3-dev \
  python3 python3-pip net-tools zlib1g-dev libsodium-dev gettext
sudo apt-get install -y cargo rustfmt protobuf-compiler

pip3 install mako

git clone https://github.com/elementsproject/lightning.git
cd lightning
git checkout v24.11.1

pip3 install grpcio-tools

./configure
make -j$(nproc)
sudo make install
