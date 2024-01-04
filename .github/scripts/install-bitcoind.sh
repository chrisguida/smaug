#!/bin/sh

set -e

DIRNAME="bitcoin-${BITCOIN_VERSION}"
FILENAME="${DIRNAME}-x86_64-linux-gnu.tar.gz"

cd /tmp/

wget "https://bitcoincore.org/bin/bitcoin-core-${BITCOIN_VERSION}/${FILENAME}"
tar -xf "${FILENAME}"
sudo mv "${DIRNAME}"/bin/* "/usr/local/bin"
rm -rf "${FILENAME}" "${DIRNAME}"

mkdir -p ~/.bitcoin
