import logging
import os
from pathlib import Path

import pytest
from pyln.testing.fixtures import *  # noqa: F403
from pyln.testing.utils import BITCOIND_CONFIG, write_config

# Define utility paths
RUST_PROFILE = os.environ.get("RUST_PROFILE", "debug")
COMPILED_PATH = Path.cwd() / "target" / RUST_PROFILE / "smaug"
DOWNLOAD_PATH = Path.cwd() / "tests" / "smaug"


def get_plugin():
    if COMPILED_PATH.is_file():
        return COMPILED_PATH
    elif DOWNLOAD_PATH.is_file():
        return DOWNLOAD_PATH
    else:
        raise ValueError("No plugin was found.")


@pytest.fixture
def bitcoind(directory, teardown_checks):  # noqa: F811
    chaind = network_daemons[env("TEST_NETWORK", "regtest")]  # noqa: F405
    bitcoind = chaind(bitcoin_dir=directory)

    BITCOIND_REGTEST = {"rpcport": BITCOIND_CONFIG["rpcport"]}

    BITCOIND_CONFIG["blockfilterindex"] = 1
    BITCOIND_REGTEST["blockfilterindex"] = 1

    conf_file = os.path.join(directory, "bitcoin.conf")
    write_config(conf_file, BITCOIND_CONFIG, BITCOIND_REGTEST)

    try:
        bitcoind.start()
    except Exception:
        bitcoind.stop()
        raise

    info = bitcoind.rpc.getnetworkinfo()

    if (
        info["version"] < 200100
        and env("TEST_NETWORK") != "liquid-regtest"  # noqa: F405
    ):
        bitcoind.rpc.stop()
        raise ValueError(
            "bitcoind is too old. At least version 20100 (v0.20.1) is "
            f"needed, current version is {info['version']}"
        )
    elif info["version"] < 160000:
        bitcoind.rpc.stop()
        raise ValueError(
            "elementsd is too old. At least version 160000 (v0.16.0) is "
            f"needed, current version is {info['version']}"
        )

    info = bitcoind.rpc.getblockchaininfo()
    if info["blocks"] < 101:
        bitcoind.generate_block(101 - info["blocks"])
    elif bitcoind.rpc.getwalletinfo()["balance"] < 1:
        logging.debug("Insufficient balance, generating 1 block")
        bitcoind.generate_block(1)

    yield bitcoind

    try:
        bitcoind.stop()
    except Exception:
        bitcoind.proc.kill()
    bitcoind.proc.wait()


@pytest.fixture
def ln_node(node_factory):  # noqa: F811
    yield node_factory.get_nodes(
        1,
        opts={
            "plugin": get_plugin(),
            "smaug_brpc_user": BITCOIND_CONFIG["rpcuser"],
            "smaug_brpc_pass": BITCOIND_CONFIG["rpcpassword"],
            "smaug_brpc_port": BITCOIND_CONFIG["rpcport"],
        },
    )[0]
