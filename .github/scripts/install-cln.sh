#!/bin/sh

set -e

sudo apt-get update
sudo apt-get install -y \
  autoconf automake build-essential git libtool libsqlite3-dev \
  python3 python3-pip net-tools zlib1g-dev libsodium-dev gettext
pip3 install --upgrade pip
pip3 install --user poetry
sudo apt-get install -y cargo rustfmt protobuf-compiler
pip3 install --upgrade pip
pip3 install mako

git clone https://github.com/niftynei/lightning.git
cd lightning
git checkout 37ad798a02336a82460b865fd4e6a29d8880856c

pip3 install -r plugins/clnrest/requirements.txt

./configure
make -j$(nproc)
sudo make install
